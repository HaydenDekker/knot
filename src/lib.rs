mod server;

pub mod adapters;
pub mod application;
pub mod domain;

// Re-export application context
pub use server::AppContext;

// Re-export subprocess runner
pub use adapters::subprocess::SubprocessAgentRunner;
pub use domain::entities::Loom;
pub use domain::value_objects::RigAgentConfig;

// Re-export server lifecycle from composition root
pub use server::{
    AppConfig, build_app_context, run_startup, start_config_pipeline,
    start_event_pipeline, start_knot, start_state_writer,
};
