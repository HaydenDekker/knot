//! Composition root wiring test.
//!
//! Verifies that `build_app_context` wires all hexagonal layers correctly.
//! Non-network test — does not spin up an HTTP server.

use std::sync::Arc;

use knot::application::ports::{
    AgentRunner, GitVersioningPort, LoomLogPort, LoomRepository, TieOffSink,
};
use knot::AppConfig;

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
    assert_eq!(ctx.rig_config.cli_path, "pi");
    assert!(ctx.rig_config.cli_args.is_empty());

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
