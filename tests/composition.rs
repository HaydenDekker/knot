//! Composition root wiring test.
//!
//! Verifies that `build_app_context` wires all hexagonal layers correctly.
//! Non-network test — does not spin up an HTTP server.

use std::path::PathBuf;
use std::sync::Arc;

use knot::application::ports::{
    AgentRunner, GitVersioningPort, LoomLogPort, LoomRepository, TieOffSink,
};
use knot::AppConfig;

/// Verify `AppConfig::with_rig_dir()` sets the custom rig directory
/// while keeping all other defaults identical to `default_config()`.
#[test]
fn app_config_with_rig_dir_uses_custom_path() {
    let custom = PathBuf::from("/tmp/my-custom-rig");
    let config = AppConfig::with_rig_dir(custom.clone());

    assert_eq!(config.rig_dir, custom);
    assert_eq!(config.rig_config.agent_adapter, knot::AgentAdapter::PiStdio);
}

/// Verify `AppConfig::with_rig_dir()` works with relative paths.
#[test]
fn app_config_with_rig_dir_accepts_relative_path() {
    let config = AppConfig::with_rig_dir("./dev-rig".into());
    assert_eq!(config.rig_dir, PathBuf::from("./dev-rig"));
}

/// Verify `AppConfig::with_rig_dir()` produces a config that wires
/// correctly through `build_app_context`.
#[test]
fn app_config_with_rig_dir_builds_app_context() {
    let tmp = std::env::temp_dir().join("knot-test-with-rig-dir");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let config = AppConfig::with_rig_dir(tmp.clone());
    let (ctx, _strand_rx, _config_rx) = knot::build_app_context(&config);

    // The rig_dir is propagated into the context
    assert_eq!(ctx.rig_dir, tmp);
    assert!(ctx.store.list().is_empty());
}

/// Verify `AppConfig::default_config()` unchanged — still defaults to `./rig`.
#[test]
fn app_config_default_config_unchanged() {
    let config = AppConfig::default_config();
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(config.rig_dir, cwd.join("rig"));
}

/// Verify `build_app_context` wires all hex layers correctly.
#[test]
fn build_app_context_wires_layers() {
    let config = AppConfig::default_config();
    let (ctx, _strand_rx, _config_rx) = knot::build_app_context(&config);

    // Store is present and empty (not yet populated)
    assert!(ctx.store.list().is_empty());

    // Ports are present (trait objects)
    let _repo: &dyn LoomRepository = &*ctx.loom_repo;
    let _log: &dyn LoomLogPort = &*ctx.loom_log_port;
    let _sink: &dyn TieOffSink = &*ctx.tie_off_sink;

    // Agent runner is present (subprocess)
    let _runner: &dyn AgentRunner = &*ctx.agent_runner;

    // Workspace config is loaded with defaults
    assert_eq!(ctx.rig_config.agent_adapter, knot::AgentAdapter::PiStdio);

    // Both event senders are present; receivers are returned for wiring
    let _ = _strand_rx;
    let _ = _config_rx;
}

/// Verify `FileSystemGitVersioner` is wired as `Arc<dyn GitVersioningPort>`
/// in the composition root (`start_event_pipeline`).
///
/// This is a compile-time check: if the types don't match, this test
/// fails to compile.
#[test]
fn git_versioner_is_trait_object_safe() {
    let versioner = knot::adapters::outbound::FileSystemGitVersioner::new(
        std::path::PathBuf::from("/tmp"),
    );
    let _obj: Arc<dyn GitVersioningPort> = Arc::new(versioner);
}
