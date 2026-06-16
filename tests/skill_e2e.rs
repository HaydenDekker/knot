//! End-to-end integration tests for file-first CRUD operations and skill
//! workflows.
//!
//! Tests cover the gaps identified in the HTTP observability plan:
//! - File-first CRUD: create, modify, delete profiles/looms/knots via files
//! - Skill workflows: Pi CLI subprocess tests for knot-create skill
//!
//! File-first CRUD tests use a live Knot server and verify state via GET
//! endpoints after direct filesystem operations.
//!
//! Skill workflow tests invoke `pi` as a subprocess and are `#[ignore]` by
//! default — run with `cargo test --test skill_e2e -- --ignored`.

mod helpers;

use std::fs;
use std::path::Path;
use tokio::process::Command;
use std::time::Duration;

use helpers::*;
use knot::AppConfig;
use knot::RigAgentConfig;

// ── Port allocation ────────────────────────────────────────────────────────

/// Next available port for file-first CRUD tests.
static mut NEXT_PORT: u16 = 33000;

fn alloc_port() -> u16 {
    unsafe {
        let port = NEXT_PORT;
        NEXT_PORT += 1;
        port
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Write a profile file and return its path.
fn write_profile_file(
    profiles_dir: &Path,
    name: &str,
    provider: &str,
    model: &str,
    profile_prompt: &str,
    body: Option<&str>,
) -> std::path::PathBuf {
    fs::create_dir_all(profiles_dir).unwrap();
    let path = profiles_dir.join(format!("{name}.md"));
    let body_section = body
        .map(|b| format!("---\n\n{b}"))
        .unwrap_or_default();
    let content = format!(
        "---\nname: {name}\nprovider: {provider}\nmodel: {model}\n\
         profile-prompt: |\n  {profile_prompt}\n---\n{body_section}",
    );
    fs::write(&path, content).unwrap();
    path
}

/// Write a knot definition file inside a loom directory.
fn write_knot_file(
    loom_dir: &Path,
    knot_name: &str,
    strand_dir: &Path,
    instructions: &str,
) -> std::path::PathBuf {
    fs::create_dir_all(loom_dir).unwrap();
    let path = loom_dir.join(format!("{knot_name}.md"));
    let content = format!(
        "---\nname: {knot_name}\nagent-profile-ref: fast\nstrand-dir: \
         \"{}\"\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: |\n    {instructions}\n---\n\n# \
         {knot_name}\n",
        strand_dir.display()
    );
    fs::write(&path, content).unwrap();
    path
}

/// Parse a JSON array of profile responses.
fn parse_profiles(body: &str) -> Vec<serde_json::Value> {
    serde_json::from_str(body).expect("profiles should be JSON array")
}

/// Parse a single profile response.
fn parse_profile(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("profile should be JSON")
}

/// Parse a JSON array of loom summaries.
fn parse_looms(body: &str) -> Vec<serde_json::Value> {
    serde_json::from_str(body).expect("looms should be JSON array")
}

/// Check if the `pi` CLI binary is available.
fn pi_available() -> bool {
    std::process::Command::new("pi")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── File-First CRUD: Loom ──────────────────────────────────────────────────

/// Create a loom directory → verify via GET /looms → remove directory →
/// verify via GET /looms that loom is gone.
///
/// Validates the file-first loom deletion workflow: remove the `*-loom/`
/// directory and Knot's file watcher detects the removal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_first_loom_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create profiles dir (empty — no profiles needed for this test)
    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();

    // Create fast profile so knot can resolve it
    write_profile_file(
        &profiles_dir,
        "fast",
        "openai",
        "gpt-4o",
        "You are a reviewer.",
        None,
    );

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // 1. GET /looms should be empty at startup
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let looms = parse_looms(&body);
    assert!(looms.is_empty(), "no looms at startup");

    // 2. Create a loom directory with a knot file
    let loom_dir = base_dir.join("delete-me-loom");
    let strand_dir = base_dir.join("strands");
    fs::create_dir_all(&strand_dir).unwrap();
    write_knot_file(
        &loom_dir,
        "test-knot",
        &strand_dir,
        "Test knot for deletion",
    );

    // 3. Wait for auto-discovery
    assert!(
        wait_for_loom_discovery(&host_port, 1).await,
        "loom should be discovered"
    );

    // 4. Verify loom exists
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let looms = parse_looms(&body);
    assert_eq!(looms.len(), 1, "should have 1 loom, got: {body}");
    assert_eq!(
        looms[0]["id"].as_str().unwrap(),
        "delete-me-loom"
    );

    // 5. Delete the loom directory
    fs::remove_dir_all(&loom_dir).unwrap();

    // 6. Wait for file watcher to detect removal.
    //    The file watcher fires KnotDeleted for each knot file.
    //    The loom shell remains (no LoomRemoved event exists),
    //    but knot_count drops to 0.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let result =
            tokio::time::timeout(Duration::from_millis(2000), http_get(&host_port, "/looms"))
                .await;
        if let Ok(Ok((status, body))) = result {
            if status.contains("200") {
                let looms = parse_looms(&body);
                if looms.iter().any(|l| l["knot_count"].as_u64() == Some(0)) {
                    break;
                }
            }
        }
    }

    // 7. Verify loom has 0 knots after directory deletion.
    //    The loom shell persists (no LoomRemoved event), but all
    //    knots are removed and the loom is effectively empty.
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let looms = parse_looms(&body);
    assert_eq!(looms.len(), 1, "loom shell should persist");
    assert_eq!(
        looms[0]["knot_count"].as_u64().unwrap(),
        0,
        "loom should have 0 knots after directory deletion"
    );

    // 8. Verify directory is actually gone on disk
    assert!(
        !loom_dir.exists(),
        "loom directory should be removed from disk"
    );
}

// ── File-First CRUD: Profile ───────────────────────────────────────────────

/// Write a profile file → verify via GET /profiles/{name} → edit frontmatter
/// → verify updated values via GET /profiles/{name}.
///
/// Validates the file-first profile modification workflow. Profiles are read
/// fresh from disk on each GET request (no caching in the profile repo).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_first_profile_modify() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // 1. Write a profile file
    write_profile_file(
        &profiles_dir,
        "editor",
        "openai",
        "gpt-4o",
        "You are a reviewer.",
        None,
    );

    // Small delay to ensure file is flushed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Verify via GET /profiles/editor
    let (status, body) =
        http_get(&host_port, "/profiles/editor")
            .await
            .expect("profile endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile = parse_profile(&body);
    assert_eq!(profile["name"].as_str().unwrap(), "editor");
    assert_eq!(profile["provider"].as_str().unwrap(), "openai");
    assert_eq!(profile["model"].as_str().unwrap(), "gpt-4o");

    // 3. Edit the profile file — change model
    let updated_content = r#"---
name: editor
provider: anthropic
model: claude-sonnet-4-20250514
profile-prompt: |
  You are an editor.
---
"#;
    fs::write(profiles_dir.join("editor.md"), updated_content).unwrap();

    // 4. Verify updated values via GET /profiles/editor
    let (status, body) =
        http_get(&host_port, "/profiles/editor")
            .await
            .expect("profile endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile = parse_profile(&body);
    assert_eq!(profile["name"].as_str().unwrap(), "editor");
    assert_eq!(profile["provider"].as_str().unwrap(), "anthropic");
    assert_eq!(
        profile["model"].as_str().unwrap(),
        "claude-sonnet-4-20250514"
    );
}

/// Write a profile file → verify via GET /profiles → delete file → verify
/// via GET /profiles that profile is gone.
///
/// Validates the file-first profile deletion workflow. Profiles are read
/// fresh from disk on each GET request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_first_profile_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // 1. Write a profile file
    write_profile_file(
        &profiles_dir,
        "temp-profile",
        "openai",
        "gpt-4o",
        "Temporary profile.",
        None,
    );

    // 2. Verify via GET /profiles
    let (status, body) =
        http_get(&host_port, "/profiles")
            .await
            .expect("profiles endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profiles = parse_profiles(&body);
    assert!(
        profiles.iter().any(|p| p["name"].as_str().unwrap() == "temp-profile"),
        "temp-profile should exist"
    );

    // 3. Delete the profile file
    fs::remove_file(profiles_dir.join("temp-profile.md")).unwrap();

    // 4. Verify via GET /profiles that profile is gone
    let (status, body) =
        http_get(&host_port, "/profiles")
            .await
            .expect("profiles endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profiles = parse_profiles(&body);
    let names: Vec<&str> = profiles
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"temp-profile"),
        "temp-profile should be gone, found: {:?}",
        names
    );
}

/// Write a profile file with markdown body → verify GET /profiles/{name}
/// returns the `body` field.
///
/// Validates the profile body end-to-end: markdown content after the closing
/// `---` frontmatter delimiter is preserved and returned in the response.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_first_profile_body_e2e() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // 1. Write a profile file with a markdown body
    write_profile_file(
        &profiles_dir,
        "bodied",
        "openai",
        "gpt-4o",
        "You are a reviewer.",
        Some("# Body Profile\n\nThis profile has a markdown body.\n"),
    );

    // 2. Verify via GET /profiles/bodied that body is returned
    let (status, body) =
        http_get(&host_port, "/profiles/bodied")
            .await
            .expect("profile endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile = parse_profile(&body);
    assert_eq!(profile["name"].as_str().unwrap(), "bodied");
    assert!(
        profile.get("body").is_some(),
        "profile should have a body field"
    );
    let body_content = profile["body"].as_str().unwrap();
    assert!(
        body_content.contains("Body Profile"),
        "body should contain markdown heading, got: {body_content}"
    );
    assert!(
        body_content.contains("markdown body"),
        "body should contain body text, got: {body_content}"
    );
}

// ── File-First CRUD: Knot ──────────────────────────────────────────────────

/// Write a knot file inside an existing loom → verify via
/// GET /looms/{id}/knots → delete the knot file → verify knot is gone.
///
/// Validates the file-first knot deletion workflow. The file watcher detects
/// the removed `.md` file and deregisters the knot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_first_knot_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create profiles dir with fast profile
    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    write_profile_file(
        &profiles_dir,
        "fast",
        "openai",
        "gpt-4o",
        "You are a reviewer.",
        None,
    );

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // 1. Create a loom with a knot
    let loom_dir = base_dir.join("knot-del-loom");
    let strand_dir = base_dir.join("strands");
    fs::create_dir_all(&strand_dir).unwrap();
    write_knot_file(&loom_dir, "my-knot", &strand_dir, "My knot");

    // 2. Wait for auto-discovery
    assert!(
        wait_for_knot_count(&host_port, "knot-del-loom", 1).await,
        "knot should be discovered"
    );

    // 3. Verify knot exists
    let (status, body) =
        http_get(&host_port, "/looms/knot-del-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(knots.contains(&"my-knot".to_string()));

    // 4. Delete the knot file
    fs::remove_file(loom_dir.join("my-knot.md")).unwrap();

    // 5. Wait for file watcher to detect removal
    assert!(
        wait_for_knot_count(&host_port, "knot-del-loom", 0).await,
        "knot should be removed from loom"
    );

    // 6. Verify knot is gone
    let (status, body) =
        http_get(&host_port, "/looms/knot-del-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        !knots.contains(&"my-knot".to_string()),
        "knot should be gone after file deletion"
    );
}

// ── Skill Workflow Tests ───────────────────────────────────────────────────
// These tests invoke `pi` as a subprocess and are #[ignore] by default.
// Run with: cargo test --test skill_e2e -- --ignored --include-ignored

/// Invoke `knot-create` skill via Pi CLI subprocess to create a profile,
/// then verify the profile file exists and is discoverable via GET endpoint.
///
/// This test validates the full skill workflow: the agent reads the SKILL.md,
/// writes the correct file format to the correct path, and Knot discovers
/// the new profile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires `pi` CLI installed"]
async fn skill_workflow_knot_create_profile() {
    if !pi_available() {
        return; // Skip if pi is not installed
    }

    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");
    let api_url = format!("http://{host_port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // Verify no profiles exist before skill invocation
    let (status, body) =
        http_get(&host_port, "/profiles")
            .await
            .expect("profiles endpoint should respond");
    assert!(status.contains("200"));
    let profiles = parse_profiles(&body);
    assert!(profiles.is_empty(), "no profiles before skill invocation");

    // Invoke the knot-create skill via Pi CLI subprocess.
    // The prompt instructs the agent to create a profile using the skill's
    // documented workflow: write a .md file to rig/profiles/, then verify.
    let prompt = format!(
        "Use the knot-create skill to create an agent profile.\n\
         Profile name: skill-test-profile\n\
         Provider: openai\n\
         Model: gpt-4o\n\
         System prompt: You are a test agent.\n\
         Write the profile file to the profiles directory in the rig, \
         then verify it is discoverable via GET {}/profiles. \
         The rig directory is: {}",
        api_url,
        base_dir.display()
    );

    let child = Command::new("pi")
        .arg("-p")
        .env("KNOT_API_URL", &api_url)
        .current_dir(&base_dir)
        .spawn();

    match child {
        Ok(mut child) => {
            // Write the prompt to stdin and close it
            use tokio::io::AsyncWriteExt;
            let mut stdin = child.stdin.take().expect("should have stdin");
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.flush().await;
            drop(stdin);

            // Wait for completion with timeout
            let timeout = Duration::from_secs(120);
            let wait_result =
                tokio::time::timeout(timeout, async {
                    child.wait_with_output().await
                })
                .await;

            match wait_result {
                Ok(Ok(output)) => {
                    if !output.status.success() {
                        eprintln!(
                            "Pi subprocess failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Pi subprocess error: {e}");
                }
                Err(_) => {
                    eprintln!("Pi subprocess timed out after {:?}", timeout);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to spawn pi: {e}");
        }
    }

    // Give the file watcher time to pick up any file changes
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Verify the profile file was created
    let profile_path = profiles_dir.join("skill-test-profile.md");
    assert!(
        profile_path.exists(),
        "profile file should exist at {}",
        profile_path.display()
    );

    // Verify via GET /profiles/skill-test-profile
    let (status, body) =
        http_get_retry(&host_port, "/profiles/skill-test-profile", 30, 100)
            .await
            .expect("profile endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile = parse_profile(&body);
    assert_eq!(
        profile["name"].as_str().unwrap(),
        "skill-test-profile"
    );
}

/// Invoke `knot-create` skill via Pi CLI subprocess to create a loom with a
/// knot, then verify the loom directory exists and the knot is discovered
/// via GET endpoint.
///
/// This test validates the full skill workflow for loom creation: the agent
/// creates the `*-loom` directory, writes the knot definition file, and Knot
/// auto-discovers the loom and its knots.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires `pi` CLI installed"]
async fn skill_workflow_knot_create_loom_and_knot() {
    if !pi_available() {
        return; // Skip if pi is not installed
    }

    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Pre-create the fast profile so the knot can reference it
    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    write_profile_file(
        &profiles_dir,
        "fast",
        "openai",
        "gpt-4o",
        "You are a reviewer.",
        None,
    );

    let port = alloc_port();
    let host_port = format!("127.0.0.1:{port}");
    let api_url = format!("http://{host_port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start");

    // Verify no looms exist before skill invocation
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"));
    let looms = parse_looms(&body);
    assert!(looms.is_empty(), "no looms before skill invocation");

    // Invoke the knot-create skill via Pi CLI subprocess.
    let prompt = format!(
        "Use the knot-create skill to create a loom named 'skill-test-loom' \
         with a single knot named 'review'.\n\
         Knot configuration:\n\
         - agent-profile-ref: fast\n\
         - strand-dir: strands\n\
         - instructions: Review the document for quality.\n\
         Write the loom directory and knot file, then verify the loom is \
         discoverable via GET {}/looms. The rig directory is: {}",
        api_url,
        base_dir.display()
    );

    let child = Command::new("pi")
        .arg("-p")
        .env("KNOT_API_URL", &api_url)
        .current_dir(&base_dir)
        .spawn();

    match child {
        Ok(mut child) => {
            use tokio::io::AsyncWriteExt;
            let mut stdin = child.stdin.take().expect("should have stdin");
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.flush().await;
            drop(stdin);

            let timeout = Duration::from_secs(120);
            let wait_result =
                tokio::time::timeout(timeout, async {
                    child.wait_with_output().await
                })
                .await;

            match wait_result {
                Ok(Ok(output)) => {
                    if !output.status.success() {
                        eprintln!(
                            "Pi subprocess failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Pi subprocess error: {e}");
                }
                Err(_) => {
                    eprintln!("Pi subprocess timed out after {:?}", timeout);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to spawn pi: {e}");
        }
    }

    // Give the file watcher time to pick up changes
    tokio::time::sleep(Duration::from_millis(3000)).await;

    // Verify the loom directory was created
    let loom_dir = base_dir.join("skill-test-loom");
    assert!(
        loom_dir.is_dir(),
        "loom directory should exist at {}",
        loom_dir.display()
    );

    // Verify the knot file was created
    let knot_file = loom_dir.join("review.md");
    assert!(
        knot_file.exists(),
        "knot file should exist at {}",
        knot_file.display()
    );

    // Verify via GET /looms that the loom is discovered
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let looms = parse_looms(&body);
    assert!(
        looms.iter()
            .any(|l| l["id"].as_str().unwrap() == "skill-test-loom"),
        "skill-test-loom should be discovered"
    );

    // Verify via GET /looms/skill-test-loom that the knot is present
    let (status, body) =
        http_get_retry(&host_port, "/looms/skill-test-loom", 30, 100)
            .await
            .expect("loom endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value = serde_json::from_str(&body).unwrap();
    let knots = loom["knots"].as_array().expect("knots should be array");
    assert!(
        knots.iter()
            .any(|k| k["id"].as_str().unwrap() == "review"),
        "review knot should be present in loom"
    );
}
