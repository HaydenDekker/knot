//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

mod all;
#[cfg(test)]
mod test_fixtures;
pub mod types;

// ── Re-export all public types from all.rs for backward compatibility ────

pub use all::ConfigEventHandler;
pub use all::DiscoverLooms;
pub use all::GetKnotStatus;
pub use all::GetLoom;
pub use all::GetLoomActivity;
pub use all::KnotAction;
pub use all::ListLooms;
pub use all::ManageKnot;
pub use all::ProcessStrand;
pub use all::ReloadConfig;
pub use all::RegisterLoom;
pub use all::UnregisterLoom;
pub use all::WriteState;

// ── Re-export shared types ──────────────────────────────────────────────

pub use types::{format_timestamp, KnotStatus, LoomSummary};
