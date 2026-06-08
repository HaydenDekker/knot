//! HTTP loom CRUD integration tests.
//!
//! Verifies loom registration, discovery, and unregistration via HTTP,
//! followed by strand processing through the event pipeline.

mod helpers;

use std::fs;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

/// `POST /looms` registers a loom with knots → create strand file in source
/// dir → tie-off produced → verify via `GET /looms/:id/knots/:knot_name`.
///
/// Verifies end-to-end: HTTP → RegisterLoom → EventSource::watch() → file
/// creation → debounce → agent → tie-off.
#[tokio::test]
async fn http_register_then_process_strand() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Mock agent script — ignores all CLI args built by ProcessStrand.
    let mock_agent = create_mock_agent(&base_dir, "http-processed");

    let port = 32020;
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

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Create source directory AFTER server start so startup discovery
    // doesn't find it — we test POST /looms registration path.
    let source_dir = base_dir.join("http-reg-loom");
    fs::create_dir(&source_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) =
        make_knot_content_with_dirs(&base_dir);
    fs::write(source_dir.join("review.md"), knot_content).unwrap();

    // 1. POST /looms to register the loom with knot definitions.
    let body = serde_json::json!({
        "id": "http-reg-loom",
        "knots": [
            {
                "name": "review-knot",
                "agent_config": {
                    "goal": "Review documents",
                    "provider": "openai",
                    "model": "gpt-4o"
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Review docs"
                },
                "strand_dir": strand_dir.to_string_lossy(),
                "tie_off_dir": tie_off_dir.to_string_lossy()
            }
        ]
    });
    eprintln!("DEBUG: about to POST /looms");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    eprintln!("DEBUG: sleep done, POSTing...");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        http_post_json(&host_port, "/looms", &body),
    )
    .await;
    let (status, _resp) = match result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => panic!("POST /looms failed: {}", e),
        Err(_) => panic!("POST /looms timed out after 10s"),
    };
    eprintln!("DEBUG: POST /looms returned: {}", status);
    assert!(
        status.contains("201"),
        "register loom should return 201, got: {status}"
    );

    // 2. Verify loom is registered and has knots.
    let (status, body) =
        http_get_retry(&host_port, "/looms/http-reg-loom", 30, 100)
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().expect("knots should be array");
    assert_eq!(knots.len(), 1, "loom should have 1 knot");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "review-knot",
        "knot id should match"
    );

    // 3. Create a strand file in the strand directory (watched by the loom).
    let strand_path = strand_dir.join("new-strand.md");
    fs::write(&strand_path, "strand content via http").unwrap();

    // Wait for debounce + processing.
    tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;

    // 4. Verify tie-off was produced in tie_off_dir.
    let tie_off_path = tie_off_dir.join("new-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("http-processed"),
        "tie-off should contain agent output, got: {content}"
    );

    // 5. Verify via GET /looms/:id/knots/:knot_name.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/http-reg-loom/knots/review-knot",
            30,
            100,
        )
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
}

/// `DELETE /looms/:id` stops processing — new strand files are NOT
/// processed after unregistration (watcher removed).
///
/// Verifies: Register → Unregister → create strand → no tie-off produced.
#[tokio::test]
async fn unregister_stops_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create source directory with knot definition file.
    let source_dir = base_dir.join("unreg-loom");
    fs::create_dir(&source_dir).unwrap();
    let (knot_content, strand_dir, _tie_off_dir) =
        make_knot_content_with_dirs(&base_dir);
    fs::write(source_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script.
    let mock_agent = create_mock_agent(&base_dir, "should-not-run");

    let port = 32022;
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

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // 1. Verify loom was discovered at startup.
    let (status, _body) =
        http_get_retry(&host_port, "/looms/unreg-loom", 30, 100)
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 2. DELETE /looms/:id to unregister the loom (stops watcher).
    let (status, _body) =
        http_delete(&host_port, "/looms/unreg-loom")
            .await
            .expect("unregister should respond");
    assert!(
        status.contains("204"),
        "unregister should return 204, got: {status}"
    );

    // Give a brief moment for the watcher to be removed.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // 3. Create a strand file AFTER unregistration.
    let strand_path = source_dir.join("post-unreg-strand.md");
    fs::write(&strand_path, "this should not be processed").unwrap();

    // Wait to confirm no processing happens.
    tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;

    // 4. Verify NO tie-off was produced.
    let tie_off_path =
        source_dir.join(".knot-output/post-unreg-strand.md.output");
    assert!(
        !tie_off_path.exists(),
        "tie-off should NOT exist after unregister: {}",
        tie_off_path.display()
    );

    // 5. Verify loom is no longer in the list.
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    let ids: Vec<_> = summaries
        .iter()
        .map(|s| s["id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        !ids.contains(&"unreg-loom"),
        "unregistered loom should not appear in list"
    );

    let _ = shutdown_tx.send(());
}
