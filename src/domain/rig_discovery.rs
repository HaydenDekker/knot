//! Rig directory discovery.
//!
//! Pure domain function — scans a directory for `*-rig` subdirectories
//! and returns a `RigDiscovery` enum describing what was found.
//! No ports, no store, no IO traits. Runs before any use cases are
//! constructed.

use std::path::{Path, PathBuf};

/// Result of scanning a directory for `*-rig` subdirectories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RigDiscovery {
    /// No `*-rig` directories found.
    None,
    /// Exactly one `*-rig` directory found.
    Single(PathBuf),
    /// Two or more `*-rig` directories found.
    /// Paths are sorted alphabetically for deterministic output.
    Multiple(Vec<PathBuf>),
    /// Explicit rig name was provided.
    /// Path is `directory/explicit_name`.
    Named(PathBuf),
}

/// Scan `directory` for `*-rig` subdirectories.
///
/// If `explicit_name` is `Some(name)`, returns `RigDiscovery::Named`
/// with the path `directory/name` (regardless of what `*-rig` dirs exist).
///
/// If `explicit_name` is `None`, scans the directory and returns:
/// - `RigDiscovery::None` — zero matches
/// - `RigDiscovery::Single(path)` — exactly one match
/// - `RigDiscovery::Multiple(paths)` — two or more matches (sorted)
pub fn discover_rigs(
    directory: &Path,
    explicit_name: Option<&str>,
) -> RigDiscovery {
    if let Some(name) = explicit_name {
        return RigDiscovery::Named(directory.join(name));
    }

    let mut rigs: Vec<PathBuf> = Vec::new();

    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return RigDiscovery::None,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with("-rig") {
                    rigs.push(path);
                }
            }
        }
    }

    rigs.sort();

    match rigs.len() {
        0 => RigDiscovery::None,
        1 => RigDiscovery::Single(rigs.pop().unwrap()),
        _ => RigDiscovery::Multiple(rigs),
    }
}
