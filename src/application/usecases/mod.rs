//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

mod all;
mod config_event_handler;
mod loom;
mod manage_knot;
mod process_strand;
pub mod query;
#[cfg(test)]
mod test_fixtures;
pub mod types;
mod write_state;

// ── Re-export all public types for backward compatibility ────

pub use config_event_handler::ConfigEventHandler;
pub use loom::DiscoverLooms;
pub use loom::ReloadConfig;
pub use loom::RegisterLoom;
pub use loom::UnregisterLoom;
pub use manage_knot::{KnotAction, ManageKnot};
pub use process_strand::ProcessStrand;
pub use query::GetKnotStatus;
pub use query::GetLoom;
pub use query::GetLoomActivity;
pub use query::ListLooms;
pub use write_state::WriteState;

// ── Re-export shared types ──────────────────────────────────────────────

pub use types::{format_timestamp, KnotStatus, LoomSummary};
