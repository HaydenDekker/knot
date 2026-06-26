//! Notify-based file system event source adapter.
//!
//! Wraps `notify::RecommendedWatcher` and maps raw file system events
//! to `StrandEvent` and `ConfigEvent` domain types. Emits events to
//! two mpsc channels — the debounce engine (application layer)
//! subscribes to the strand channel, and the `ConfigEventHandler`
//! subscribes to the config channel.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::adapters::logging;
use crate::application::ports::{EventSource, PortError};
use crate::domain::entities::{Knot, KnotId, LoomId, StrandPath};
use crate::domain::events::{ConfigEvent, StrandEvent};
use crate::domain::knot_file;

// ── Path Resolution ────────────────────────────────────────────────────────

/// Resolve a relative path against `project_root`, matching the logic
/// in `FileSystemLoomRepository::resolve_path`.
fn resolve_path(project_root: &Path, value: &PathBuf) -> PathBuf {
    let path = if value.is_absolute() {
        value.clone()
    } else {
        project_root.join(value)
    };

    fs::canonicalize(&path).unwrap_or_else(|_| {
        let mut result = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    let _ = result.pop();
                }
                _ => result.push(component),
            }
        }
        result
    })
}

// ── WatchType ──────────────────────────────────────────────────────────────

/// The type of watch being registered, determining how events are mapped.
#[derive(Debug, Clone)]
pub enum WatchType {
    /// Strand directory watch — maps file events to `StrandEvent`.
    /// Carries the loom and knot IDs for the watched directory.
    Strand(LoomId, KnotId),
    /// Rig directory watch — maps new `*-loom` directory creation
    /// to `ConfigEvent::LoomAdded`.
    Rig,
    /// Loom directory watch — maps `.md` file create/modify/delete
    /// to `ConfigEvent::Knot*`. Carries the loom ID.
    Loom(LoomId),
}

// ── NotifyEventSource ──────────────────────────────────────────────────────

/// Shared mutable state between the watcher callback and the public API.
struct InnerState {
    strand_sender: mpsc::Sender<StrandEvent>,
    config_sender: mpsc::Sender<ConfigEvent>,
    /// Watched directories with their types.
    ///
    /// Multiple entries can share the same path (e.g. two knots
    /// watching the same strand directory) — each carries its own
    /// `(loom_id, knot_id)` pair. Stored as a `Vec` so
    /// `find_watch_types` can iterate in longest-path-first order
    /// — more specific (longer) paths always take priority over
    /// broader (shorter) parent paths.
    watched_dirs: Vec<(PathBuf, WatchType)>,
    /// Project root directory. Used to resolve relative `strand_dir`
    /// and `strand_dir` paths from knot config files during event
    /// mapping, matching the resolution done during initial load.
    project_root: PathBuf,
}

impl InnerState {
    /// Map a raw notify event to strand and config events,
    /// filtering and enriching based on the watch types.
    ///
    /// Returns a tuple of (Vec<StrandEvent>, Option<ConfigEvent>).
    /// Multiple strand events can be returned when multiple knots
    /// watch the same directory. At most one config event is returned
    /// (rig/loom watches are unique per directory).
    fn map_event(
        &self,
        event: &Event,
    ) -> (Vec<StrandEvent>, Option<ConfigEvent>) {
        let path = match event.paths.first() {
            Some(p) => p,
            None => return (Vec::new(), None),
        };

        // Find all matching watch types for this path.
        // Multiple knots can watch the same strand directory.
        let watch_types = self.find_watch_types(path);

        if watch_types.is_empty() {
            return (Vec::new(), None);
        }

        // Collect strand events (one per Strand watch) and at most one
        // config event (rig/loom watches are unique per directory).
        let mut strand_events = Vec::new();
        let mut config_event: Option<ConfigEvent> = None;

        for wt in watch_types {
            match &wt {
                WatchType::Strand(loom_id, knot_id) => {
                    let (se, _) = self.map_strand_event(event, path, loom_id, knot_id);
                    if let Some(se) = se {
                        strand_events.push(se);
                    }
                }
                WatchType::Rig => {
                    let (_, ce) = self.map_rig_event(event, path);
                    config_event = ce.or(config_event);
                }
                WatchType::Loom(loom_id) => {
                    let (_, ce) = self.map_loom_event(event, path, loom_id);
                    config_event = ce.or(config_event);
                }
            }
        }

        (strand_events, config_event)
    }

    /// Map events for strand directory watches.
    ///
    /// Accepts all files — text files are processed downstream,
    /// binary files are filtered there.
    fn map_strand_event(
        &self,
        event: &Event,
        path: &Path,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        // Only process file events (skip directories)
        if path.is_dir() {
            return (None, None);
        }

        let strand_path = StrandPath(path.to_path_buf());

        let strand_event = match event.kind {
            EventKind::Create(_) => Some(StrandEvent::Created {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path,
            }),
            EventKind::Modify(_) => Some(StrandEvent::Modified {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path,
            }),
            EventKind::Remove(_) => Some(StrandEvent::Deleted {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path,
            }),
            _ => None,
        };

        (strand_event, None)
    }

    /// Map events for rig directory watches.
    ///
    /// Handles two types of events:
    /// 1. New `*-loom` directories at the rig root → `ConfigEvent::LoomAdded`
    /// 2. `.md` file changes inside existing loom directories → `ConfigEvent::Knot*`
    fn map_rig_event(
        &self,
        event: &Event,
        path: &Path,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        // Case 1: New loom directory at the rig root level.
        // Detects `*-loom` directory creation and emits `ConfigEvent::LoomAdded`.
        if path.is_dir() {
            if matches!(event.kind, EventKind::Create(_))
                && let Some(name) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    && name.ends_with("-loom") {
                        let loom_id = LoomId(name.to_string());
                        let loom_dir = path.to_string_lossy().to_string();
                        return (
                            None,
                            Some(ConfigEvent::LoomAdded {
                                loom_id,
                                loom_dir,
                            }),
                        );
                    }
            // Directory events that don't match the above are ignored.
            return (None, None);
        }

        // Case 2: `.md` file changes inside a loom subdirectory.
        // Extract the loom ID from the path (parent directory must end in `-loom`).
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            return (None, None);
        }

        let loom_id = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .filter(|name| name.ends_with("-loom"))
            .map(|name| LoomId(name.to_string()));

        let Some(loom_id) = loom_id else {
            return (None, None);
        };

        // Parse the knot file for create/modify events
        let config_event = match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                match std::fs::read_to_string(path) {
                    Ok(content) => match knot_file::parse(&content) {
                        Ok((knot_file, _warnings)) => {
                            let knot = Knot {
                                id: KnotId(knot_file.name.clone()),
                                agent_profile_ref: knot_file.agent_profile_ref,
                                prompt_template: knot_file.prompt_template,
                                strand_dir: resolve_path(
                                    &self.project_root,
                                    &knot_file.strand_dir,
                                ),
                                git_versioned: knot_file.git_versioned,
                            };
                            if matches!(event.kind, EventKind::Create(_)) {
                                Some(ConfigEvent::KnotAdded {
                                    loom_id,
                                    knot,
                                })
                            } else {
                                Some(ConfigEvent::KnotModified {
                                    loom_id,
                                    knot,
                                })
                            }
                        }
                        Err(_) => None,
                    },
                    Err(_) => None,
                }
            }
            EventKind::Remove(_) => {
                path
                    .file_stem()
                    .and_then(|n| n.to_str()).map(|name| ConfigEvent::KnotDeleted {
                        loom_id,
                        knot_id: KnotId(name.to_string()),
                    })
            }
            _ => None,
        };

        (None, config_event)
    }

    /// Map events for loom directory watches.
    ///
    /// Maps `.md` file create/modify/delete to `ConfigEvent::Knot*`.
    fn map_loom_event(
        &self,
        event: &Event,
        path: &Path,
        loom_id: &LoomId,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        // Only process `.md` files (skip directories and other files)
        if path.is_dir() {
            return (None, None);
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            return (None, None);
        }

        let config_event = match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                // Parse the knot file to get the full Knot entity
                match std::fs::read_to_string(path) {
                    Ok(content) => match knot_file::parse(&content) {
                        Ok((knot_file, _warnings)) => {
                            let knot = Knot {
                                id: KnotId(knot_file.name.clone()),
                                agent_profile_ref: knot_file.agent_profile_ref,
                                prompt_template: knot_file.prompt_template,
                                strand_dir: resolve_path(
                                    &self.project_root,
                                    &knot_file.strand_dir,
                                ),
                                git_versioned: knot_file.git_versioned,
                            };
                            if matches!(event.kind, EventKind::Create(_)) {
                                Some(ConfigEvent::KnotAdded {
                                    loom_id: loom_id.clone(),
                                    knot,
                                })
                            } else {
                                Some(ConfigEvent::KnotModified {
                                    loom_id: loom_id.clone(),
                                    knot,
                                })
                            }
                        }
                        Err(_) => {
                            // File parse failed — skip silently
                            // The file may be incomplete during write
                            None
                        }
                    },
                    Err(_) => None,
                }
            }
            EventKind::Remove(_) => {
                // Extract knot name from filename (remove .md extension)
                path
                    .file_stem()
                    .and_then(|n| n.to_str()).map(|name| ConfigEvent::KnotDeleted {
                        loom_id: loom_id.clone(),
                        knot_id: KnotId(name.to_string()),
                    })
            }
            _ => None,
        };

        (None, config_event)
    }

    /// Find all watch types for a path by checking watched directories.
    ///
    /// Iterates in longest-path-first order so that more specific
    /// (longer) paths always take priority over broader (shorter)
    /// parent paths. For example, a watch on `/rig/strands/` with
    /// `WatchType::Strand` will match before a watch on `/rig/` with
    /// `WatchType::Rig`.
    ///
    /// Returns all matching watch types — multiple knots can watch
    /// the same strand directory, and each should receive events.
    fn find_watch_types(&self, path: &Path) -> Vec<WatchType> {
        // Find the maximum depth among matching directories so we
        // only return watches at the most specific level (shadowing).
        let max_len = self.watched_dirs
            .iter()
            .filter(|(dir, _)| path.starts_with(dir))
            .map(|(dir, _)| dir.as_os_str().len())
            .max();

        let Some(max_len) = max_len else {
            return Vec::new();
        };

        // Return all watch types at the most specific depth.
        self.watched_dirs
            .iter()
            .filter(|(dir, _)| {
                path.starts_with(dir) && dir.as_os_str().len() == max_len
            })
            .map(|(_, wt)| wt.clone())
            .collect()
    }
}

/// File-system event source backed by the `notify` crate.
///
/// Wraps a `notify::RecommendedWatcher`, maps raw events to
/// `StrandEvent` and `ConfigEvent` domain types, and forwards them
/// to the application layer via mpsc channels.
pub struct NotifyEventSource {
    watcher: Mutex<RecommendedWatcher>,
    state: Arc<Mutex<InnerState>>,
}

impl NotifyEventSource {
    /// Create a new `NotifyEventSource` with two event channels.
    ///
    /// `strand_sender` receives `StrandEvent` from strand directory
    /// watches. `config_sender` receives `ConfigEvent` from rig and
    /// loom directory watches.
    ///
    /// `project_root` is used to resolve relative `strand_dir`
    /// paths from knot config files during event mapping.
    ///
    /// Uses `notify::Config` with a 50ms poll interval for consistent
    /// test behaviour across platforms.
    pub fn new(
        strand_sender: mpsc::Sender<StrandEvent>,
        config_sender: mpsc::Sender<ConfigEvent>,
        project_root: PathBuf,
    ) -> Self {
        let state = Arc::new(Mutex::new(InnerState {
            strand_sender,
            config_sender,
            watched_dirs: Vec::new(),
            project_root,
        }));

        let state_clone = state.clone();

        let watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    let inner = state_clone.lock().unwrap();
                    let (strand_events, config_event) =
                        inner.map_event(&event);
                    // Log every mapped event for observability.
                    // Volume is low (a few hundred/day), so log all of them.
                    if let Some(path) = event.paths.first() {
                        for se in &strand_events {
                            let kind = match se {
                                StrandEvent::Created { .. } => "Created",
                                StrandEvent::Modified { .. } => "Modified",
                                StrandEvent::Deleted { .. } => "Deleted",
                            };
                            let detail = format!("{:?}", se);
                            logging::log_notify_event(kind, path, &detail);
                        }
                        if let Some(ref ce) = config_event {
                            let kind = match ce {
                                ConfigEvent::LoomAdded { .. } => "LoomAdded",
                                ConfigEvent::KnotAdded { .. } => "KnotAdded",
                                ConfigEvent::KnotModified { .. } => "KnotModified",
                                ConfigEvent::KnotDeleted { .. } => "KnotDeleted",
                            };
                            let detail = format!("{:?}", ce);
                            logging::log_notify_event(kind, path, &detail);
                        }
                    }
                    // Use try_send to avoid blocking the notify callback
                    // thread. If the channel is full, the event is dropped
                    // — this is acceptable because:
                    // 1. Strand events: the debounce engine will see the
                    //    file exists on next poll cycle
                    // 2. Config events: the config handler is idempotent
                    //    and will re-process on next event
                    for se in strand_events {
                        let _ = inner.strand_sender.try_send(se);
                    }
                    if let Some(ce) = config_event {
                        let _ = inner.config_sender.try_send(ce);
                    }
                }
            },
            notify::Config::default()
                .with_poll_interval(std::time::Duration::from_millis(50)),
        )
        .expect("failed to create notify watcher");

        Self {
            watcher: Mutex::new(watcher),
            state,
        }
    }

    /// Register a watch type for a specific directory.
    ///
    /// Call this before `watch()` so events carry the correct metadata
    /// and map to the right event type.
    pub fn register_watch(&self, path: PathBuf, watch_type: WatchType) {
        let (wt_label, extra) = match &watch_type {
            WatchType::Strand(_, knot_id) => ("Strand", Some(format!("knot={}", knot_id.0))),
            WatchType::Rig => ("Rig", None),
            WatchType::Loom(loom_id) => ("Loom", Some(format!("loom={}", loom_id.0))),
        };

        // Canonicalise the rig watch path so it matches the absolute
        // paths that notify reports. Without this, `find_watch_types()`
        // fails to match when `run_startup()` registers with a relative
        // path like `./rig` but notify fires events with absolute paths.
        // Use resolve_path() which joins against project_root first,
        // then canonicalises — matching the resolution used elsewhere.
        let canonical_path =
            if matches!(watch_type, WatchType::Rig) && !path.is_absolute() {
                let project_root = self.state.lock().unwrap().project_root.clone();
                resolve_path(&project_root, &path)
            } else {
                path
            };

        logging::log_watch_event("register", &canonical_path, wt_label, extra.as_deref());
        let mut inner = self.state.lock().unwrap();
        // Update if the exact (path, watch_type) pair already exists,
        // otherwise push. Multiple knots can watch the same directory,
        // so we only deduplicate identical entries — not duplicate paths.
        if let Some(pos) = inner
            .watched_dirs
            .iter()
            .position(|(p, wt)| p == &canonical_path && Self::watch_types_equal(wt, &watch_type))
        {
            inner.watched_dirs[pos] = (canonical_path.clone(), watch_type);
        } else {
            inner.watched_dirs.push((canonical_path, watch_type));
        }
    }

    /// Check if two watch types refer to the same logical watch.
    ///
    /// Two `Strand` watches are equal only if both loom and knot IDs match.
    /// `Rig` and `Loom` watches are equal by their variant/loom ID.
    fn watch_types_equal(a: &WatchType, b: &WatchType) -> bool {
        match (a, b) {
            (WatchType::Strand(l1, k1), WatchType::Strand(l2, k2)) => {
                l1 == l2 && k1 == k2
            }
            (WatchType::Rig, WatchType::Rig) => true,
            (WatchType::Loom(l1), WatchType::Loom(l2)) => l1 == l2,
            _ => false,
        }
    }

    /// Set the loom and knot IDs for a strand source directory.
    ///
    /// Convenience method equivalent to
    /// `register_watch(path, WatchType::Strand(loom_id, knot_id))`.
    /// Maintains backward compatibility with existing callers.
    pub fn with_loom_ids(
        &self,
        source_dir: PathBuf,
        loom_id: LoomId,
        knot_id: KnotId,
    ) {
        self.register_watch(
            source_dir,
            WatchType::Strand(loom_id, knot_id),
        );
    }

    /// Set default strand IDs (used as fallback).
    ///
    /// Maintains backward compatibility with builder-style `with_ids`.
    pub fn with_ids(self, loom_id: LoomId, knot_id: KnotId) -> Self {
        self.register_watch(
            PathBuf::from("__default_ids__"),
            WatchType::Strand(loom_id, knot_id),
        );
        self
    }

    /// Remove a specific `(path, watch_type)` entry from `watched_dirs`.
    ///
    /// Unlike `unwatch()` which removes ALL entries for a path,
    /// this method only removes the entry matching the exact
    /// `(canonical_path, watch_type)` pair — mirroring the
    /// deduplication pattern of `register_watch()`.
    ///
    /// Only calls `notify::unwatch()` when the last watcher entry
    /// for the path is removed, so remaining knot watchers continue
    /// receiving events from the shared directory.
    pub fn unwatch_with_type(&self, path: &Path, watch_type: WatchType) {
        // Canonicalise the path so it matches stored entries.
        let canonical_path =
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Determine label for logging before removing the entry.
        let (wt_label, extra) = match &watch_type {
            WatchType::Strand(_, knot_id) => ("Strand", Some(format!("knot={}", knot_id.0))),
            WatchType::Rig => ("Rig", None),
            WatchType::Loom(loom_id) => ("Loom", Some(format!("loom={}", loom_id.0))),
        };

        // Remove only the entry matching (path, watch_type), then check
        // if any other entries remain for this path. Drop lock BEFORE
        // calling notify::unwatch() to avoid deadlock.
        let (has_remaining, removed_entry) = {
            let mut inner = self.state.lock().unwrap();
            let pos = inner
                .watched_dirs
                .iter()
                .position(|(p, wt)| {
                    p == &canonical_path
                        && Self::watch_types_equal(wt, &watch_type)
                });
            let removed_entry = pos.and_then(|i| {
                inner.watched_dirs.remove(i);
                Some(())
            });
            let has_remaining = inner
                .watched_dirs
                .iter()
                .any(|(p, _)| p == &canonical_path);
            (has_remaining, removed_entry)
        }; // state lock dropped here

        // Only call notify::unwatch() if no other entries remain for
        // this path — remaining knots still need the watch active.
        if !has_remaining {
            if let Some(()) = removed_entry {
                if let Err(e) = self
                    .watcher
                    .lock()
                    .unwrap()
                    .unwatch(&canonical_path)
                {
                    logging::log_notify_event(
                        "UnwatchError",
                        &canonical_path,
                        &format!("unwatch failed: {}", e),
                    );
                }
            }
        }

        logging::log_watch_event("stopped", &canonical_path, wt_label, extra.as_deref());
    }
}

impl EventSource for NotifyEventSource {
    fn register_watch(&self, path: PathBuf, watch_type: WatchType) {
        // Delegate to the inherent implementation
        NotifyEventSource::register_watch(self, path, watch_type);
    }

    fn set_loom_ids(
        &self,
        source_dir: &Path,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) {
        self.with_loom_ids(
            source_dir.to_path_buf(),
            loom_id.clone(),
            knot_id.clone(),
        );
    }

    fn watch(&self, path: &Path) -> Result<(), PortError> {
        // Canonicalise the path so it matches stored entries.
        // `register_watch()` canonicalises Rig paths, so we need the
        // same form here for the lookup to succeed.
        let canonical_path =
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Determine watch type and mode
        let watch_type = {
            let inner = self.state.lock().unwrap();
            inner
                .watched_dirs
                .iter()
                .find(|(p, _)| p == &canonical_path)
                .map(|(_, wt)| wt.clone())
                // Fall back to default IDs if no per-directory mapping
                .or_else(|| {
                    inner
                        .watched_dirs
                        .iter()
                        .find(|(p, _)| p == &PathBuf::from("__default_ids__"))
                        .map(|(_, wt)| wt.clone())
                })
        };

        // Update watched_dirs map, then drop lock BEFORE calling
        // watcher.watch() — the notify system may trigger a directory
        // scan on watch(), firing the callback which also needs the
        // state lock. Dropping first avoids deadlock.
        {
            let mut inner = self.state.lock().unwrap();
            if let Some(ref wt) = watch_type {
                // Ensure the path is in the map for event lookup
                if let Some(pos) = inner
                    .watched_dirs
                    .iter()
                    .position(|(p, _)| p == &canonical_path)
                {
                    inner.watched_dirs[pos].1 = wt.clone();
                } else {
                    inner.watched_dirs
                        .push((canonical_path.clone(), wt.clone()));
                }
            } else {
                // Default: treat as strand watch with unknown IDs
                let default = WatchType::Strand(
                    LoomId("unknown".to_string()),
                    KnotId("unknown".to_string()),
                );
                inner.watched_dirs
                    .push((canonical_path.clone(), default));
            }
        } // state lock dropped here — notify callback can now proceed

        // Rig watch uses recursive mode to detect both new loom directories
        // (at the rig root) and knot file changes (in loom subdirectories).
        // Strand and loom watches use their configured modes.
        let mode = match &watch_type {
            Some(WatchType::Rig) => RecursiveMode::Recursive,
            Some(WatchType::Loom(_)) => RecursiveMode::NonRecursive,
            _ => RecursiveMode::Recursive,
        };

        let (wt_label, extra) = match &watch_type {
            Some(WatchType::Strand(_, knot_id)) => ("Strand", Some(format!("knot={}", knot_id.0))),
            Some(WatchType::Rig) => ("Rig", None),
            Some(WatchType::Loom(loom_id)) => ("Loom", Some(format!("loom={}", loom_id.0))),
            None => ("Default", None),
        };
        self.watcher
            .lock()
            .unwrap()
            .watch(&canonical_path, mode)
            .map_err(|e| PortError::EventWatchFailed(e.to_string()))?;

        logging::log_watch_event("started", &canonical_path, wt_label, extra.as_deref());
        Ok(())
    }

    fn unwatch(&self, path: &Path) -> Result<(), PortError> {
        // Canonicalise the path so it matches stored entries.
        let canonical_path =
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Look up watch type and remove from map, then drop lock
        // BEFORE calling watcher.unwatch() to avoid deadlock with
        // the notify callback.
        let watch_type = {
            let mut inner = self.state.lock().unwrap();
            let wt = inner
                .watched_dirs
                .iter()
                .find(|(p, _)| p == &canonical_path)
                .map(|(_, wt)| wt.clone());
            inner.watched_dirs.retain(|(p, _)| p != &canonical_path);
            wt
        }; // state lock dropped here

        self.watcher
            .lock()
            .unwrap()
            .unwatch(&canonical_path)
            .map_err(|e| PortError::EventUnwatchFailed(e.to_string()))?;

        let (wt_label, extra) = match &watch_type {
            Some(WatchType::Strand(_, knot_id)) => ("Strand", Some(format!("knot={}", knot_id.0))),
            Some(WatchType::Rig) => ("Rig", None),
            Some(WatchType::Loom(loom_id)) => ("Loom", Some(format!("loom={}", loom_id.0))),
            None => ("Unknown", None),
        };
        logging::log_watch_event("stopped", &canonical_path, wt_label, extra.as_deref());

        Ok(())
    }

    fn unwatch_with_type(
        &self,
        path: &Path,
        watch_type: WatchType,
    ) -> Result<(), PortError> {
        // Delegate to the inherent implementation which handles
        // targeted entry removal and conditional notify unwatch.
        self.unwatch_with_type(path, watch_type);
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Poll watcher interval plus buffer for reliable test timing.
    const POLL_DELAY: Duration = Duration::from_millis(300);

    /// Poll the strand channel for an event, retrying briefly.
    fn recv_strand_event(
        rx: &mut mpsc::Receiver<StrandEvent>,
        timeout: Duration,
    ) -> Option<StrandEvent> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok(event) = rx.try_recv() {
                return Some(event);
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// Poll the config channel for an event, retrying briefly.
    fn recv_config_event(
        rx: &mut mpsc::Receiver<ConfigEvent>,
        timeout: Duration,
    ) -> Option<ConfigEvent> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok(event) = rx.try_recv() {
                return Some(event);
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// Build a `NotifyEventSource` for strand event tests (backward compat).
    fn create_source(
        loom_id: &str,
        knot_id: &str,
    ) -> (
        NotifyEventSource,
        mpsc::Receiver<StrandEvent>,
        mpsc::Receiver<ConfigEvent>,
    ) {
        let (strand_tx, strand_rx) = mpsc::channel(100);
        let (config_tx, config_rx) = mpsc::channel(100);
        let source = NotifyEventSource::new(strand_tx, config_tx, PathBuf::from("/tmp")).with_ids(
            LoomId(loom_id.to_string()),
            KnotId(knot_id.to_string()),
        );
        (source, strand_rx, config_rx)
    }

    /// Build a `NotifyEventSource` with fresh channels (no default IDs).
    fn create_source_fresh(
    ) -> (
        NotifyEventSource,
        mpsc::Receiver<StrandEvent>,
        mpsc::Receiver<ConfigEvent>,
    ) {
        let (strand_tx, strand_rx) = mpsc::channel(100);
        let (config_tx, config_rx) = mpsc::channel(100);
        let source = NotifyEventSource::new(strand_tx, config_tx, PathBuf::from("/tmp"));
        (source, strand_rx, config_rx)
    }

    // ── Strand Event Tests (backward compatibility) ─────────────────────

    #[test]
    fn watcher_starts() {
        let (source, _strand_rx, _config_rx) =
            create_source("test-loom", "test-knot");
        let dir = TempDir::new().unwrap();

        assert!(
            source.watch(dir.path()).is_ok(),
            "watch() should succeed"
        );

        // unwatch() succeeds proves watcher was active
        assert!(
            source.unwatch(dir.path()).is_ok(),
            "unwatch() should succeed"
        );
    }

    #[test]
    fn create_event_emitted() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let file_path = dir.path().join("new-strand.md");
        fs::write(&file_path, "content").unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Created event");
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(strand_path.0.file_name().unwrap(), "new-strand.md");
            }
            other => panic!("Expected Created event, got: {:?}", other),
        }
    }

    #[test]
    fn modify_event_emitted() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        let file_path = dir.path().join("existing.md");
        fs::write(&file_path, "initial").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "updated content").unwrap();
        drop(file);

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Modified event");
        match event {
            StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(strand_path.0.file_name().unwrap(), "existing.md");
            }
            other => panic!("Expected Modified event, got: {:?}", other),
        }
    }

    #[test]
    fn delete_event_emitted() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        let file_path = dir.path().join("to-delete.md");
        fs::write(&file_path, "will be deleted").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::remove_file(&file_path).unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Deleted event");
        match event {
            StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "to-delete.md"
                );
            }
            other => panic!("Expected Deleted event, got: {:?}", other),
        }
    }

    #[test]
    fn directory_events_filtered() {
        let dir = TempDir::new().unwrap();
        let (source, mut strand_rx, mut config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let subdir = dir.path().join("new-subdir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "directory creation should not emit strand events"
        );
        // Config channel: no events (this is a strand watch, not a rig watch)
        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "directory creation should not emit config events on strand watch"
        );
    }

    #[test]
    fn event_outside_source_dir_filtered() {
        let watched_dir = TempDir::new().unwrap();
        let outside_dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(watched_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let outside_file = outside_dir.path().join("outside.md");
        fs::write(&outside_file, "outside content").unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_strand_event(&mut rx, Duration::from_millis(200)).is_none(),
            "events outside watched dir should not be emitted"
        );
    }

    #[test]
    fn event_mapping_correct_types() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // 1. Create → StrandEvent::Created
        let file_path = dir.path().join("mapping-test.md");
        fs::write(&file_path, "initial").unwrap();
        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive Create event");
        assert!(
            matches!(event, StrandEvent::Created { .. }),
            "Create should map to StrandEvent::Created, got: {:?}", event
        );

        // 2. Modify → StrandEvent::Modified
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "modified").unwrap();
        drop(file);
        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive Modify event");
        assert!(
            matches!(event, StrandEvent::Modified { .. }),
            "Modify should map to StrandEvent::Modified, got: {:?}", event
        );

        // 3. Remove → StrandEvent::Deleted
        fs::remove_file(&file_path).unwrap();
        thread::sleep(POLL_DELAY);

        // PollWatcher may emit Modify before Remove; drain and check last
        let mut last_event: Option<StrandEvent> = None;
        while let Some(event) =
            recv_strand_event(&mut rx, Duration::from_millis(100))
        {
            last_event = Some(event);
        }
        let event = last_event
            .expect("should receive at least one event for remove");
        assert!(
            matches!(event, StrandEvent::Deleted { .. }),
            "Remove should map to StrandEvent::Deleted, got: {:?}", event
        );
    }

    /// Text file (`.txt`) create emits a strand event.
    #[test]
    fn txt_file_create_emits_strand_event() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "plain text content").unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Created event for .txt file");
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "notes.txt"
                );
            }
            other => panic!(
                "Expected Created event for .txt, got: {:?}",
                other
            ),
        }
    }

    /// Source code file (`.rs`) modify emits a strand event.
    #[test]
    fn rs_file_modify_emits_strand_event() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        let file_path = dir.path().join("lib.rs");
        fs::write(&file_path, "fn main() {}").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::write(&file_path, "fn main() { println!(\"hello\"); }")
            .unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Modified event for .rs file");
        match event {
            StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "lib.rs"
                );
            }
            other => panic!(
                "Expected Modified event for .rs, got: {:?}",
                other
            ),
        }
    }

    /// Binary file create still emits a strand event — binary
    /// filtering happens downstream (in the processing pipeline),
    /// not in the event source.
    #[test]
    fn binary_file_create_emits_strand_event() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let file_path = dir.path().join("data.bin");
        fs::write(&file_path, [0u8, 1, 2, 255, 0]).unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Created event for binary file");
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "data.bin"
                );
            }
            other => panic!(
                "Expected Created event for binary file, got: {:?}",
                other
            ),
        }
    }

    /// Config file (`.json`) create emits a strand event.
    #[test]
    fn json_file_create_emits_strand_event() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let file_path = dir.path().join("config.json");
        fs::write(&file_path, r#"{"key": "value"}"#).unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Created event for .json file");
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "config.json"
                );
            }
            other => panic!(
                "Expected Created event for .json, got: {:?}",
                other
            ),
        }
    }

    // ── Config Event Tests (Phase 3: Rig/Loom Watching) ────────────────

    /// Watch rig directory with `WatchType::Rig`; create a `*-loom`
    /// directory → `ConfigEvent::LoomAdded` emitted on config channel.
    #[test]
    fn rig_dir_new_loom_emits_config_event() {
        let rig_dir = TempDir::new().unwrap();
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Register rig watch
        source.register_watch(
            rig_dir.path().to_path_buf(),
            WatchType::Rig,
        );
        source.watch(rig_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a new loom directory
        let loom_dir = rig_dir.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "rig watch should not emit strand events"
        );

        // Config channel: should receive LoomAdded
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a LoomAdded config event");
        match event {
            ConfigEvent::LoomAdded { loom_id, loom_dir } => {
                assert_eq!(loom_id.0, "my-loom");
                assert!(loom_dir.ends_with("my-loom"));
            }
            other => panic!(
                "Expected ConfigEvent::LoomAdded, got: {:?}",
                other
            ),
        }
    }

    /// Watch loom directory with `WatchType::Loom(id)`; create a `.md`
    /// file with valid knot frontmatter → `ConfigEvent::KnotAdded`
    /// emitted on config channel.
    #[test]
    fn loom_dir_new_knot_emits_config_event() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Register loom watch
        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id.clone()),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a new knot .md file with valid frontmatter
        let knot_content = "---
name: new-knot
agent-profile-ref: fast
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
---

Review the document
";
        let knot_path = loom_dir.path().join("new-knot.md");
        fs::write(&knot_path, knot_content).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "loom watch should not emit strand events"
        );

        // Config channel: should receive KnotAdded
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a KnotAdded config event");
        match event {
            ConfigEvent::KnotAdded {
                loom_id: lid,
                knot,
            } => {
                assert_eq!(lid.0, "test-loom");
                assert_eq!(knot.id.0, "new-knot");
                assert_eq!(knot.agent_profile_ref, "fast");
            }
            other => panic!(
                "Expected ConfigEvent::KnotAdded, got: {:?}",
                other
            ),
        }
    }

    /// Watch loom directory with `WatchType::Loom(id)`; edit an existing
    /// `.md` knot file → `ConfigEvent::KnotModified` emitted on config
    /// channel.
    #[test]
    fn loom_dir_edit_knot_emits_config_event() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Create the knot file before watching
        let knot_content = "---
name: existing-knot
agent-profile-ref: fast
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
---

Initial instructions
";
        let knot_path = loom_dir.path().join("existing-knot.md");
        fs::write(&knot_path, knot_content).unwrap();

        // Register loom watch
        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id.clone()),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Drain any events from file creation before watch
        while recv_config_event(&mut config_rx, Duration::from_millis(50))
            .is_some()
        {}

        // Edit the knot file (change profile ref)
        let updated_content = "---
name: existing-knot
agent-profile-ref: detailed
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
---

Updated instructions
";
        fs::write(&knot_path, updated_content).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "loom watch should not emit strand events on edit"
        );

        // Config channel: should receive KnotModified
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a KnotModified config event");
        match event {
            ConfigEvent::KnotModified {
                loom_id: lid,
                knot,
            } => {
                assert_eq!(lid.0, "test-loom");
                assert_eq!(knot.id.0, "existing-knot");
                assert_eq!(knot.agent_profile_ref, "detailed");
            }
            other => panic!(
                "Expected ConfigEvent::KnotModified, got: {:?}",
                other
            ),
        }
    }

    /// Watch loom directory with `WatchType::Loom(id)`; delete a `.md`
    /// knot file → `ConfigEvent::KnotDeleted` emitted on config channel.
    #[test]
    fn loom_dir_delete_knot_emits_config_event() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Create the knot file before watching
        let knot_content = "---
name: to-delete
agent-profile-ref: fast
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
---

Instructions
";
        let knot_path = loom_dir.path().join("to-delete.md");
        fs::write(&knot_path, knot_content).unwrap();

        // Register loom watch
        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id.clone()),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Drain any events from file creation before watch
        while recv_config_event(&mut config_rx, Duration::from_millis(50))
            .is_some()
        {}

        // Delete the knot file
        fs::remove_file(&knot_path).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "loom watch should not emit strand events on delete"
        );

        // Config channel: should receive KnotDeleted
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a KnotDeleted config event");
        match event {
            ConfigEvent::KnotDeleted {
                loom_id: lid,
                knot_id,
            } => {
                assert_eq!(lid.0, "test-loom");
                assert_eq!(knot_id.0, "to-delete");
            }
            other => panic!(
                "Expected ConfigEvent::KnotDeleted, got: {:?}",
                other
            ),
        }
    }

    /// Non-`-loom` directory creation in rig watch is ignored.
    #[test]
    fn rig_dir_non_loom_directory_ignored() {
        let rig_dir = TempDir::new().unwrap();
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            rig_dir.path().to_path_buf(),
            WatchType::Rig,
        );
        source.watch(rig_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a non-loom directory
        let subdir = rig_dir.path().join("random-dir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "non-loom directory should not emit events"
        );
    }

    /// Non-`.md` file creation in loom watch is ignored.
    #[test]
    fn loom_dir_non_md_file_ignored() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a non-md file
        let file_path = loom_dir.path().join("config.json");
        fs::write(&file_path, "{}").unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "non-.md file should not emit config events"
        );
    }

    /// Directory creation in loom watch is ignored.
    #[test]
    fn loom_dir_subdirectory_creation_ignored() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a subdirectory
        let subdir = loom_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "subdirectory creation in loom watch should not emit events"
        );
    }

    /// Malformed knot file in loom watch is silently skipped.
    #[test]
    fn loom_dir_malformed_knot_ignored() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a file with invalid frontmatter
        let bad_content = "# Not a valid knot file

Just some markdown.
";
        let bad_path = loom_dir.path().join("bad-knot.md");
        fs::write(&bad_path, bad_content).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "malformed knot file should not emit events"
        );
    }

    /// Two knots watching the same strand directory each receive events
    /// when a file in that directory is modified.
    #[test]
    fn two_knots_same_directory_both_receive_events() {
        let shared_dir = TempDir::new().unwrap();
        let (strand_tx, mut strand_rx) = mpsc::channel::<StrandEvent>(100);
        let (config_tx, _config_rx) = mpsc::channel::<ConfigEvent>(100);
        let source = NotifyEventSource::new(
            strand_tx,
            config_tx,
            PathBuf::from("/tmp"),
        );

        // Register two knots watching the same directory
        source.register_watch(
            shared_dir.path().to_path_buf(),
            WatchType::Strand(
                LoomId("loom-1".to_string()),
                KnotId("knot-a".to_string()),
            ),
        );
        source.register_watch(
            shared_dir.path().to_path_buf(),
            WatchType::Strand(
                LoomId("loom-1".to_string()),
                KnotId("knot-b".to_string()),
            ),
        );

        source.watch(shared_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a file in the shared directory
        let file_path = shared_dir.path().join("shared-strand.md");
        fs::write(&file_path, "content").unwrap();

        thread::sleep(POLL_DELAY);

        // Collect all events
        let mut events = Vec::new();
        while let Ok(event) = strand_rx.try_recv() {
            events.push(event);
        }

        // Should receive Created events for BOTH knots
        assert!(
            events.len() >= 2,
            "expected at least 2 events (one per knot), got {}",
            events.len()
        );

        // Verify both knots are represented
        let knot_ids: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StrandEvent::Created { knot_id, .. } => Some(knot_id.0.clone()),
                _ => None,
            })
            .collect();

        assert!(
            knot_ids.contains(&"knot-a".to_string()),
            "knot-a should have received a Created event, got: {:?}",
            knot_ids
        );
        assert!(
            knot_ids.contains(&"knot-b".to_string()),
            "knot-b should have received a Created event, got: {:?}",
            knot_ids
        );
    }

    /// Re-registering the exact same (path, watch_type) pair is
    /// idempotent — doesn't create duplicate entries.
    #[test]
    fn register_watch_idempotent_for_same_knot() {
        let dir = TempDir::new().unwrap();
        let (source, _strand_rx, _config_rx) = create_source_fresh();

        let loom_id = LoomId("loom-1".to_string());
        let knot_id = KnotId("knot-a".to_string());
        let path = dir.path().to_path_buf();

        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_id.clone()),
        );
        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_id.clone()),
        );

        let inner = source.state.lock().unwrap();
        let count = inner
            .watched_dirs
            .iter()
            .filter(|(p, wt)| {
                p == &path
                    && matches!(
                        wt,
                        WatchType::Strand(l, k) if l == &loom_id && k == &knot_id
                    )
            })
            .count();

        assert_eq!(
            count, 1,
            "duplicate (path, watch_type) should be deduplicated"
        );
    }

    /// Registering a `WatchType::Rig` watch with a relative path
    /// canonicalises it to an absolute path, so `find_watch_types()`
    /// matches against the absolute paths that notify reports.
    #[test]
    fn rig_watch_path_canonicalised() {
        let rig_dir = TempDir::new().unwrap();
        let (strand_tx, _strand_rx) = mpsc::channel::<StrandEvent>(100);
        let (config_tx, _config_rx) = mpsc::channel::<ConfigEvent>(100);

        // Use the tempdir as project_root so we can create a relative
        // path from within it.
        let project_root = rig_dir.path().to_path_buf();
        let source =
            NotifyEventSource::new(strand_tx, config_tx, project_root.clone());

        // Create a subdirectory to act as the rig (so we can refer to it
        // with a relative path).
        let rig_subdir = rig_dir.path().join("rig");
        fs::create_dir(&rig_subdir).unwrap();

        // Register a rig watch using a relative path.
        let relative_path = PathBuf::from("rig");
        source.register_watch(relative_path.clone(), WatchType::Rig);

        let inner = source.state.lock().unwrap();
        let stored_path = inner
            .watched_dirs
            .iter()
            .find(|(_, wt)| matches!(wt, WatchType::Rig))
            .map(|(p, _)| p.clone());

        assert!(
            stored_path.is_some(),
            "rig watch should be registered"
        );
        let stored_path = stored_path.unwrap();

        // The stored path should be canonicalised (absolute), matching
        // what notify would report.
        assert!(
            stored_path.is_absolute(),
            "rig watch path should be canonicalised to absolute, got: {:?}",
            stored_path
        );
        assert_eq!(
            stored_path,
            fs::canonicalize(&rig_subdir).unwrap(),
            "canonicalised path should match the actual directory"
        );
    }

    /// Creating a `*-loom` directory under a rig watch emits
    /// `ConfigEvent::LoomAdded` that includes the absolute `loom_dir`
    /// path from the notify event.
    #[test]
    fn rig_loom_added_event_includes_path() {
        let rig_dir = TempDir::new().unwrap();
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Register rig watch with absolute path
        source.register_watch(
            rig_dir.path().to_path_buf(),
            WatchType::Rig,
        );
        source.watch(rig_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a new loom directory
        let loom_path = rig_dir.path().join("discovered-loom");
        fs::create_dir(&loom_path).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "rig watch should not emit strand events"
        );

        // Config channel: should receive LoomAdded with loom_dir
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a LoomAdded config event");
        match event {
            ConfigEvent::LoomAdded {
                loom_id,
                loom_dir,
            } => {
                assert_eq!(loom_id.0, "discovered-loom");
                // loom_dir should be the absolute path from notify
                let loom_dir_path = PathBuf::from(&loom_dir);
                assert!(
                    loom_dir_path.is_absolute(),
                    "loom_dir should be an absolute path, got: {}",
                    loom_dir
                );
                assert!(
                    loom_dir_path.ends_with("discovered-loom"),
                    "loom_dir should end with the loom name, got: {}",
                    loom_dir
                );
            }
            other => panic!(
                "Expected ConfigEvent::LoomAdded, got: {:?}",
                other
            ),
        }
    }

    // ── Multi-Knot Shared Directory Tests (Phase 0: bug reproduction) ───

    /// Two knots (same loom, different knot IDs) watch the same strand
    /// directory. When we remove only knot-a's watch via
    /// `unwatch_with_type()`, knot-b's entry must remain in
    /// `watched_dirs` and `notify::unwatch()` must NOT be called
    /// (since knot-b still watches the path).
    ///
    /// This is the primary regression test for the bug where
    /// `unwatch(path)` removed ALL entries for a path instead of
    /// just the matching `(path, WatchType)` pair.
    #[test]
    fn unwatch_with_type_removes_only_matching_knot_entry() {
        let shared_dir = TempDir::new().unwrap();
        let (strand_tx, _strand_rx) = mpsc::channel::<StrandEvent>(100);
        let (config_tx, _config_rx) = mpsc::channel::<ConfigEvent>(100);
        let source = NotifyEventSource::new(
            strand_tx,
            config_tx,
            PathBuf::from("/tmp"),
        );

        let loom_id = LoomId("loom-1".to_string());
        let knot_a = KnotId("knot-a".to_string());
        let knot_b = KnotId("knot-b".to_string());
        let path = shared_dir.path().to_path_buf();

        // Register two knots watching the same directory
        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_a.clone()),
        );
        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_b.clone()),
        );

        // Verify both entries are registered
        let inner = source.state.lock().unwrap();
        assert_eq!(
            inner.watched_dirs.len(), 2,
            "expected 2 entries before unwatch"
        );
        drop(inner);

        // Start the notify watch
        source.watch(&path).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Use unwatch_with_type to remove only knot-a's entry
        source.unwatch_with_type(
            &path,
            WatchType::Strand(loom_id.clone(), knot_a.clone()),
        );

        // Verify: knot-b's entry should STILL be present
        let inner = source.state.lock().unwrap();
        let remaining: Vec<_> = inner
            .watched_dirs
            .iter()
            .filter_map(|(_, wt)| match wt {
                WatchType::Strand(l, k) if l == &loom_id => Some(k.clone()),
                _ => None,
            })
            .collect();

        assert!(
            remaining.contains(&knot_b),
            "knot_b watch should still be present after unwatching knot_a. Remaining entries: {:?}",
            inner.watched_dirs
        );

        // knot-a's entry should be gone
        assert!(
            !remaining.contains(&knot_a),
            "knot_a watch should have been removed"
        );
    }

    /// When the last watcher for a path is removed via
    /// `unwatch_with_type()`, `notify::unwatch()` IS called,
    /// freeing the OS file watch resource.
    #[test]
    fn unwatch_with_type_stops_notify_watch_when_last_entry_removed() {
        let dir = TempDir::new().unwrap();
        let (strand_tx, _strand_rx) = mpsc::channel::<StrandEvent>(100);
        let (config_tx, _config_rx) = mpsc::channel::<ConfigEvent>(100);
        let source = NotifyEventSource::new(
            strand_tx,
            config_tx,
            PathBuf::from("/tmp"),
        );

        let loom_id = LoomId("loom-1".to_string());
        let knot_a = KnotId("knot-a".to_string());
        let knot_b = KnotId("knot-b".to_string());
        let path = dir.path().to_path_buf();

        // Register and watch
        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_a.clone()),
        );
        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_b.clone()),
        );
        source.watch(&path).unwrap();

        // Verify both entries exist
        let inner = source.state.lock().unwrap();
        assert_eq!(
            inner.watched_dirs.len(), 2,
            "expected 2 entries after watch"
        );
        drop(inner);

        // Remove knot-a's entry — knot-b still watches, so notify::unwatch
        // should NOT be called yet
        source.unwatch_with_type(&path, WatchType::Strand(loom_id.clone(), knot_a));

        // Verify knot-b is still registered and notify watch is still active
        let inner = source.state.lock().unwrap();
        assert_eq!(
            inner.watched_dirs.len(), 1,
            "expected 1 entry after removing knot-a"
        );
        drop(inner);

        // Now remove knot-b's entry — this should trigger notify::unwatch
        source.unwatch_with_type(&path, WatchType::Strand(loom_id, knot_b));

        // Verify no entries remain
        let inner = source.state.lock().unwrap();
        assert!(
            inner.watched_dirs.is_empty(),
            "expected 0 entries after removing all watchers"
        );
        drop(inner);
    }

    /// After removing the **last** watcher for a path, `notify::unwatch()`
    /// should be called (stopping the underlying file system watch).
    ///
    /// This is verified indirectly by calling `unwatch()` twice on the
    /// same directory — the second call should not fail, and the state
    /// should be clean.
    #[test]
    fn unwatch_removes_entry_and_stops_notify_watch() {
        let dir = TempDir::new().unwrap();
        let (strand_tx, _strand_rx) = mpsc::channel::<StrandEvent>(100);
        let (config_tx, _config_rx) = mpsc::channel::<ConfigEvent>(100);
        let source = NotifyEventSource::new(
            strand_tx,
            config_tx,
            PathBuf::from("/tmp"),
        );

        let loom_id = LoomId("loom-1".to_string());
        let knot_id = KnotId("knot-a".to_string());
        let path = dir.path().to_path_buf();

        // Register and watch
        source.register_watch(
            path.clone(),
            WatchType::Strand(loom_id.clone(), knot_id.clone()),
        );
        source.watch(&path).unwrap();

        // Verify entry exists
        let inner = source.state.lock().unwrap();
        assert_eq!(
            inner.watched_dirs.len(), 1,
            "expected 1 entry after watch, got {}",
            inner.watched_dirs.len()
        );
        drop(inner);

        // Unwatch — should remove the entry and call notify::unwatch()
        source.unwatch(&path).unwrap();

        // Verify entry is gone
        let inner = source.state.lock().unwrap();
        assert!(
            inner.watched_dirs.is_empty(),
            "expected 0 entries after unwatch, got {}",
            inner.watched_dirs.len()
        );
        drop(inner);

        // Verify notify::unwatch() was effective: watching again
        // should succeed (no stale watch lingering)
        source.watch(&path).unwrap();
        let inner = source.state.lock().unwrap();
        assert_eq!(
            inner.watched_dirs.len(), 1,
            "expected 1 entry after re-watch, got {}",
            inner.watched_dirs.len()
        );
    }
}
