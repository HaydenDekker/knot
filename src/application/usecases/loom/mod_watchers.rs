//! Shared watcher helper for loom use cases.
//!
//! Extracted from the copy-pasted `ensure_strand_dir_and_watch` methods
//! in `DiscoverLooms`, `RegisterLoom`, and `ConfigEventHandler`.

use std::path::Path;

use crate::adapters::logging;
use crate::application::ports::{EventSource, LoomLogPort, PortError};
use crate::domain::entities::{KnotId, LoomId};
use crate::domain::events::LoomEvent;

use super::super::types::format_timestamp;

/// Ensure `strand_dir` exists on disk, then start the file watcher.
///
/// If the directory is missing, creates it (including any parent
/// directories), logs a `LoomEvent::DirectoryCreated` event, and
/// emits a log line. The watcher is always started regardless of
/// whether creation was needed.
pub(crate) fn ensure_strand_dir_and_watch(
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_dir: &Path,
    log_port: &dyn LoomLogPort,
    event_source: &dyn EventSource,
) -> Result<(), PortError> {
    let dir_created = if !strand_dir.exists() {
        std::fs::create_dir_all(strand_dir).map_err(|e| {
            PortError::LoomSaveFailed(format!(
                "failed to create strand dir '{}': {}",
                strand_dir.display(),
                e,
            ))
        })?;
        log_port.append(LoomEvent::DirectoryCreated {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            directory: strand_dir.display().to_string(),
            timestamp: format_timestamp(),
        })?;
        logging::log_knot_event(
            "dir-created",
            &loom_id.0,
            &knot_id.0,
            &format!("auto-created strand dir: {}", strand_dir.display()),
        );
        true
    } else {
        false
    };

    event_source.set_loom_ids(strand_dir, loom_id, knot_id);
    event_source.watch(strand_dir).map_err(|e| {
        PortError::LoomSaveFailed(format!(
            "failed to watch '{}': {}",
            strand_dir.display(),
            e,
        ))
    })?;

    if dir_created {
        logging::log_knot_event(
            "watch-started",
            &loom_id.0,
            &knot_id.0,
            "watcher started on newly created dir",
        );
    }

    Ok(())
}
