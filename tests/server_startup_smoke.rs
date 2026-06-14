//! Minimal smoke test for server startup — isolates the bind/listen path.
mod helpers;
use std::fs;
use knot::AppConfig;
use knot::RigAgentConfig;
use helpers::*;

/// Minimal test: create a rig dir, start server, verify port opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_starts_and_listens() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = tmp.path().join("rig");
    fs::create_dir(&rig).unwrap();

    let config = AppConfig {
        rig_dir: rig,
        bind_addr: "127.0.0.1:35500".parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    let host_port = "127.0.0.1:35500";
    wait_for_port(host_port, 5000)
        .await
        .expect("server should start listening");

    let (status, _body) = http_get(host_port, "/health")
        .await
        .expect("health endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
}

/// Same as above but with a loom directory (triggers discovery + watch).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_starts_with_loom_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    let loom_dir = rig.join("test-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, _strand_dir) = make_knot_content_with_dirs(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_fast_profile(&rig);

    let config = AppConfig {
        rig_dir: rig,
        bind_addr: "127.0.0.1:35501".parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    let host_port = "127.0.0.1:35501";
    wait_for_port(host_port, 5000)
        .await
        .expect("server should start listening");

    let (status, _body) = http_get(host_port, "/health")
        .await
        .expect("health endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
}

/// Two servers sequentially — verify no resource leak between runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_servers_sequential() {
    let tmp1 = tempfile::tempdir().unwrap();
    let rig1 = tmp1.path().join("rig");
    fs::create_dir(&rig1).unwrap();

    let config1 = AppConfig {
        rig_dir: rig1,
        bind_addr: "127.0.0.1:35502".parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle1 = spawn_server(config1);
    wait_for_port("127.0.0.1:35502", 5000)
        .await
        .expect("first server should start");
    let (status, _) = http_get("127.0.0.1:35502", "/health").await.unwrap();
    assert!(status.contains("200"));
    drop(_handle1);

    let tmp2 = tempfile::tempdir().unwrap();
    let rig2 = tmp2.path().join("rig");
    fs::create_dir(&rig2).unwrap();

    let config2 = AppConfig {
        rig_dir: rig2,
        bind_addr: "127.0.0.1:35503".parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let _handle2 = spawn_server(config2);
    wait_for_port("127.0.0.1:35503", 5000)
        .await
        .expect("second server should start");
    let (status, _) = http_get("127.0.0.1:35503", "/health").await.unwrap();
    assert!(status.contains("200"));
}
