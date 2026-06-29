//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::outbound::event_source::WatchType;
use crate::adapters::logging;
use crate::application::ports::{
    AgentProfileRepository, AgentRunner, EventSource,
    GitVersioningPort, KnotEventType, LoomLogPort, LoomRepository,
    PortError, RigLogPort, StateWriterPort, TieOffSink,
};
use crate::application::session_resume;
use crate::application::store::LoomStore;
use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, RigState, RigStateKnot, RigStateLoom,
    RigStateProfile, StrandPath, TieOff, TieOffPath,
};
use crate::domain::events::{ConfigEvent, LoomEvent, StrandEvent};
use crate::domain::knot_file::derive_tieoff_path;
use crate::domain::value_objects::{AgentConfig, RigAgentConfig};

// Re-export shared types from types module
use super::types::format_timestamp;

