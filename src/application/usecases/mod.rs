//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

mod all;
mod loom;
pub mod query;
#[cfg(test)]
mod test_fixtures;
pub mod types;

// ── Re-export all public types for backward compatibility ────

pub use all::ConfigEventHandler;
pub use all::KnotAction;
pub use all::ManageKnot;
pub use all::ProcessStrand;
pub use loom::DiscoverLooms;
pub use loom::ReloadConfig;
pub use loom::RegisterLoom;
pub use loom::UnregisterLoom;
pub use all::WriteState;
pub use query::GetKnotStatus;
pub use query::GetLoom;
pub use query::GetLoomActivity;
pub use query::ListLooms;

// ── Re-export shared types ──────────────────────────────────────────────

pub use types::{format_timestamp, KnotStatus, LoomSummary};
