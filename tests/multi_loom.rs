//! Multi-loom isolation and per-knot source directory tests.
//!
//! Verifies that multiple looms operate independently and that knots within
//! a single loom can each watch separate source directories.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

/// Two looms with different source dirs and tie-off points.
///
/// 1. Create strand in loom A → tie-off in A's point only
/// 2. Create strand in loom B → tie-off in B's point only
/// 3. No cross-interference (A's knots don't process B's strands)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiple_looms_independent() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Loom A with its own strand directory
    let loom_a_dir = base_dir.join("loom-a-loom");
    fs::create_dir(&loom_a_dir).unwrap();
    let strand_dir_a = base_dir.join("loom-a-strands");
    fs::create_dir_all(&strand_dir_a).unwrap();
    let knot_a_content = format!(
        "---\nname: review-knot\nagent-profile-ref: fast\nstrand-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review A's documents.\n---\n",
        strand_dir_a.display()
    );
    fs::write(loom_a_dir.join("review.md"), knot_a_content).unwrap();

    // Loom B with its own strand directory
    let loom_b_dir = base_dir.join("loom-b-loom");
    fs::create_dir(&loom_b_dir).unwrap();
    let strand_dir_b = base_dir.join("loom-b-strands");
    fs::create_dir_all(&strand_dir_b).unwrap();
    let knot_b_content = format!(
        "---\nname: review-knot\nagent-profile-ref: fast\nstrand-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review B's documents.\n---\n",
        strand_dir_b.display()
    );
    fs::write(loom_b_dir.join("review.md"), knot_b_content).unwrap();

    let port = 31994;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
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

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Verify both looms are registered
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100).await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 2, "should have 2 looms");

    // Collect loom IDs from the response
    let loom_ids: Vec<_> =
        summaries.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(
        loom_ids.contains(&"loom-a-loom"),
        "loom-a-loom should be registered"
    );
    assert!(
        loom_ids.contains(&"loom-b-loom"),
        "loom-b-loom should be registered"
    );

    // 1. Create strand in loom A
    let strand_a_path = strand_dir_a.join("strand-a.md");
    fs::write(&strand_a_path, "content for A").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    // Tie-off appears only in A's output directory
    let tie_off_a = base_dir.join("tie-offs/loom-a-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_a.exists(),
        "tie-off should exist in loom A: {}",
        tie_off_a.display()
    );

    // 2. Create strand in loom B
    let strand_b_path = strand_dir_b.join("strand-b.md");
    fs::write(&strand_b_path, "content for B").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    // Tie-off appears only in B's output directory
    let tie_off_b = base_dir.join("tie-offs/loom-b-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_b.exists(),
        "tie-off should exist in loom B: {}",
        tie_off_b.display()
    );

    // 3. No cross-interference
    // A's tie-off dir should NOT contain B's strand output
    let tie_off_dir_a = base_dir.join("tie-offs/loom-a-loom/review-knot");
    let files_in_a: Vec<_> =
        fs::read_dir(&tie_off_dir_a)
            .expect("should read tie-off dir A")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        !files_in_a.iter().any(|f| f.contains("strand-b")),
        "loom A should not contain loom B's strand output, got {files_in_a:?}"
    );

    // B's tie-off dir should NOT contain A's strand output
    let tie_off_dir_b = base_dir.join("tie-offs/loom-b-loom/review-knot");
    let files_in_b: Vec<_> =
        fs::read_dir(&tie_off_dir_b)
            .expect("should read tie-off dir B")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        !files_in_b.iter().any(|f| f.contains("strand-a")),
        "loom B should not contain loom A's strand output, got {files_in_b:?}"
    );

}

/// Two knots in one loom, each with its own source directory.
///
/// 1. Create a rig with a loom containing two knot files.
/// 2. Each knot defines its own `strand-dir` pointing to a separate dir.
/// 3. Start the server with a mock agent.
/// 4. Create a strand in knot A's source → processed by knot A only.
/// 5. Create a strand in knot B's source → processed by knot B only.
/// 6. Verify both knots reach `completed` status independently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_starts_with_per_knot_source_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory (contains knot definitions).
    let loom_dir = rig.join("multi-knot-loom");
    fs::create_dir(&loom_dir).unwrap();

    // External source directories for each knot.
    let source_a = root.join("source-a");
    fs::create_dir(&source_a).unwrap();
    let source_b = root.join("source-b");
    fs::create_dir(&source_b).unwrap();

    // Knot A — watches source-a.
    let knot_a_content = format!(
        "---
name: knot-a
agent-profile-ref: fast
strand-dir: \"{}\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review A\"
---
",
        source_a.display()
    );
    fs::write(loom_dir.join("knot-a.md"), knot_a_content).unwrap();

    // Knot B — watches source-b.
    let knot_b_content = format!(
        "---
name: knot-b
agent-profile-ref: fast
strand-dir: \"{}\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review B\"
---
",
        source_b.display()
    );
    fs::write(loom_dir.join("knot-b.md"), knot_b_content).unwrap();

    // Create a "fast" agent profile
    let profiles_dir = rig.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\nprofile-prompt: |\n  You are a reviewer.\n---\n\nFast Profile\n",
    )
    .unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand.
    let mock_agent = create_mock_agent(root, "processed");

    let port = 32010;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Verify loom is discovered with 2 knots.
    let (status, body) =
        http_get_retry(&host_port, "/looms/multi-knot-loom", 30, 100).await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().expect("knots should be array");
    assert_eq!(knots.len(), 2, "loom should have 2 knots");

    // Verify both knots are present.
    let knot_ids: Vec<_> = knots
        .iter()
        .map(|k| k["id"].as_str().unwrap())
        .collect();
    assert!(
        knot_ids.contains(&"knot-a"),
        "knot-a should be present"
    );
    assert!(
        knot_ids.contains(&"knot-b"),
        "knot-b should be present"
    );

    // 1. Create a strand in source-a → should trigger knot-a.
    let strand_a_path = source_a.join("strand-a.md");
    fs::write(&strand_a_path, "content for A").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    // Verify knot-a reaches completed status.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/multi-knot-loom/knots/knot-a",
            30,
            100,
        )
        .await
        .expect("knot-a status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_a_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_a_status["status"].as_str().unwrap(),
        "completed",
        "knot-a status should be completed"
    );

    // 2. Create a strand in source-b → should trigger knot-b.
    let strand_b_path = source_b.join("strand-b.md");
    fs::write(&strand_b_path, "content for B").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    // Verify knot-b reaches completed status.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/multi-knot-loom/knots/knot-b",
            30,
            100,
        )
        .await
        .expect("knot-b status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_b_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_b_status["status"].as_str().unwrap(),
        "completed",
        "knot-b status should be completed"
    );

    // Verify knot-a strand_path references source-a file.
    assert!(
        knot_a_status["last_strand_path"]
            .as_str()
            .unwrap_or("")
            .contains("strand-a.md"),
        "knot-a should reference strand-a.md, got: {knot_a_status:?}"
    );

    // Verify knot-b strand_path references source-b file.
    assert!(
        knot_b_status["last_strand_path"]
            .as_str()
            .unwrap_or("")
            .contains("strand-b.md"),
        "knot-b should reference strand-b.md, got: {knot_b_status:?}"
    );

}
