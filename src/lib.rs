mod server;

pub mod adapters;
pub mod application;
pub mod domain;

// Re-export inbound adapter types
pub use adapters::inbound::{build_app, AppContext};
pub use adapters::inbound::system::{health, list_agents};
pub use adapters::subprocess::SubprocessAgentRunner;
pub use domain::entities::Loom;
pub use domain::value_objects::RigAgentConfig;

// Re-export server lifecycle from composition root
pub use server::{
    AppConfig, build_app_context, run_startup, start_config_pipeline,
    start_event_pipeline, start_server, start_server_with_shutdown,
    ShutdownSignal,
};
