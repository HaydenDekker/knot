//! Inbound HTTP adapter.
//!
//! Handlers are thin — they extract parameters from the HTTP request and
//! delegate to application-layer use cases. They never touch ports or
//! outbound adapters directly.

pub mod loom;
pub mod router;
pub mod system;
pub mod types;

pub use loom::{
    create_knot, delete_knot, get_knot_status, get_loom, get_loom_activity,
    get_loom_knots, list_looms, register_loom, unregister_loom, update_knot,
};
pub use router::build_app;
pub use system::{get_rig_config, health, list_agents};
pub use types::{AppContext, KnotRequest, RegisterLoomRequest, RigConfigResponse};
