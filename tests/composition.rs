//! Composition root wiring test.
//!
//! Verifies that `build_app_context` wires all hexagonal layers correctly.
//! Non-network test — does not spin up an HTTP server.

use knot::application::ports::{
    AgentRunner, LoomLogPort, LoomRepository, TieOffSink,
};
use knot::AppConfig;

/// Verify `build_app_context` wires all hex layers correctly.
#[test]
fn build_app_context_wires_layers() {
    let config = AppConfig::default_config();
    let (ctx, _event_rx) = knot::build_app_context(&config);

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

    // Event sender is present; receiver is returned for pipeline wiring
    // (Receiver type proves the channel was created)
    let _ = _event_rx;
}
