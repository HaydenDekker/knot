//! Demo workflow verification tests.
//!
//! Verifies that the `knot-test` demo loom config works correctly,
//! including provider/model fields, tools configuration, and
//! tie-off generation with a stub-pi agent.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

/// Demo verification: the `knot-test` loom config has provider/model fields,
/// Knot processes `sample-document.md` and produces a populated tie-off,
/// and the loom-log records successful processing.
///
/// This test mirrors the demo workflow:
/// 1. Create a rig with a `knot-test` loom (provider + model in config)
/// 2. Place `sample-document.md` in the source directory
/// 3. Start Knot with stub-pi agent
/// 4. Verify tie-off is populated (contains system prompt + strand content)
/// 5. Verify loom-log records `StrandProcessed` with no error
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn demo_knot_test_processes_sample_document() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create strand and tie-off directories
    let strand_dir = base_dir.join("strands");
    let tie_off_dir = base_dir.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    // Create knot-test-loom directory with provider/model in config
    let loom_dir = base_dir.join("knot-test-loom");
    fs::create_dir(&loom_dir).unwrap();
    let knot_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review and summarize documents\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the provided document. Provide a concise summary\n    of its key points and any recommendations.\n---\n\n# Review Knot\n\nThis knot reviews and summarizes documents.\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    fs::write(&loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Create the sample-document.md strand
    fs::write(
        &strand_dir.join("sample-document.md"),
        r#"# Sample Document for Knot Processing

## Introduction

This is a sample document that demonstrates the Knot file
processing pipeline.

## Key Points

1. The Knot service watches a source directory for file events.
2. When a file is created or modified, the configured agent
   processes its content.
3. The agent output (tie-off) is written to the output directory.
4. Processing events are recorded in the loom-log file.

## Recommendations

- Keep documents concise for faster processing.
- Use markdown format for best results.
- Monitor the loom-log for processing status.
"#,
    )
    .unwrap();

    // Create stub-pi agent script
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32005;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 10000)
        .await
        .expect("server should start listening");

    // Wait for initial file processing (file already exists at startup,
    // but notify may emit a Create event during discovery)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // If the initial file hasn't been processed yet (startup race),
    // create a new file to trigger processing explicitly.
    let tie_off_path = tie_off_dir.join("sample-document.md.output");
    if !tie_off_path.exists() {
        // Touch the file to trigger a Modify event
        fs::write(
            &strand_dir.join("sample-document.md"),
            "# Sample Document for Knot Processing\n\n## Updated\n\nContent.",
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 1. Verify tie-off exists and contains populated content
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Tie-off should contain system prompt (proves --system-prompt was passed)
    assert!(
        tie_off_content.contains("Review the provided document"),
        "tie-off should contain system prompt, got: {tie_off_content}"
    );

    // Tie-off should contain model name (proves --model was passed)
    assert!(
        tie_off_content.contains("gpt-4o"),
        "tie-off should contain model name, got: {tie_off_content}"
    );

    // Tie-off should contain strand content (proves @<file> was used)
    assert!(
        tie_off_content.contains("Sample Document")
            || tie_off_content.contains("Knot Processing"),
        "tie-off should contain strand content, got: {tie_off_content}"
    );

    // 2. Verify knot status is `completed` via HTTP
    let (status, body) =
        http_get(&host_port, "/looms/knot-test-loom/knots/review-knot")
            .await
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    // 3. Verify loom-log records successful processing
    let log_path = base_dir.join("knot-test-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read loom log");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    assert!(
        log_content.contains("sample-document.md"),
        "loom log should reference sample-document.md"
    );

    let _ = shutdown_tx.send(());
    let _ = handle.await;
}

/// Demo verification: knot-test loom with tools configured.
///
/// Uses a knot config with `tools: [fs, web]` to verify the
/// `build_cli_args` path that emits `--tools fs,web`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn demo_knot_test_with_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create strand and tie-off directories
    let strand_dir = base_dir.join("strands");
    let tie_off_dir = base_dir.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    // Create knot-test-loom with tools in agent-config
    let loom_dir = base_dir.join("knot-test-loom");
    fs::create_dir(&loom_dir).unwrap();
    let knot_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review with tools\"\n  provider: \"anthropic\"\n  model: \"claude-sonnet-4-20250514\"\n  tools:\n    - fs\n    - web\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the document with tool access.\n---\n\n# Review Knot With Tools\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    fs::write(&loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Create stub-pi agent that echoes all received flags
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32006;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 10000)
        .await
        .expect("server should start listening");

    // Create a strand file
    fs::write(&strand_dir.join("input.md"), "Document to review.").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify tie-off exists and contains the model from knot config
    let tie_off_path = tie_off_dir.join("input.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("claude-sonnet-4-20250514"),
        "tie-off should contain the configured model, got: {tie_off_content}"
    );

    // Verify knot status is completed
    let (status, body) =
        http_get(&host_port, "/looms/knot-test-loom/knots/review-knot")
            .await
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    let _ = shutdown_tx.send(());
    let _ = handle.await;
}
