//! Shim rig — externalised workflow rig resolution.
//!
//! A **shim rig** is a project-local rig whose workflow (looms, knot
//! files, agent profiles) is served from an **external rig** — a
//! directory named by the `external-rig` key in the local rig's
//! `.workspace-agent-config.yaml`. The local rig keeps only what is
//! project-specific: the config file itself, `models.yml`, instruction
//! links, and (via the project root) the runtime tree.
//!
//! This module holds the pure resolution contract — no IO beyond
//! existence/type/readability checks on the resolved path:
//!
//! - **Local path only** — absolute, `~/...` (expanded from `$HOME`),
//!   or relative to the **project root** (the parent of the local rig
//!   directory). The service never fetches; a git checkout is a valid
//!   target, but updating it is a user/script concern.
//! - **Whole-rig import** — the resolved directory is the effective
//!   rig; there is no per-loom or per-profile selection.
//! - **Hard error** — an unresolvable path is a startup failure, never
//!   a silent fallback to the local rig.

use std::path::{Path, PathBuf};

/// Errors produced while resolving an `external-rig` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalRigError {
    /// The key is present but empty or whitespace-only.
    Empty,
    /// `~/...` was used but `$HOME` is not set.
    HomeUnset,
    /// The resolved path does not exist.
    NotFound(PathBuf),
    /// The resolved path exists but is not a directory.
    NotADirectory(PathBuf),
    /// The resolved path is a directory but cannot be read.
    Unreadable(PathBuf, String),
}

impl std::fmt::Display for ExternalRigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalRigError::Empty => {
                write!(f, "'external-rig' is empty (expected a directory path)")
            }
            ExternalRigError::HomeUnset => write!(
                f,
                "'external-rig' uses a '~/...' path but $HOME is not set"
            ),
            ExternalRigError::NotFound(p) => {
                write!(f, "external rig '{}' does not exist", p.display())
            }
            ExternalRigError::NotADirectory(p) => {
                write!(f, "external rig '{}' is not a directory", p.display())
            }
            ExternalRigError::Unreadable(p, e) => {
                write!(f, "external rig '{}' is not readable: {e}", p.display())
            }
        }
    }
}

impl std::error::Error for ExternalRigError {}

/// Resolve an `external-rig` value to an effective rig directory,
/// reading `$HOME` for `~/...` expansion.
pub fn resolve_external_rig(
    raw: &str,
    project_root: &Path,
) -> Result<PathBuf, ExternalRigError> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
    resolve_external_rig_in(raw, project_root, home.as_deref())
}

/// Resolve an `external-rig` value with an explicit `$HOME` value
/// (`None` = unset). The injectable form of [`resolve_external_rig`]
/// — used by tests so no process-global env is touched.
pub fn resolve_external_rig_in(
    raw: &str,
    project_root: &Path,
    home: Option<&str>,
) -> Result<PathBuf, ExternalRigError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ExternalRigError::Empty);
    }

    let candidate: PathBuf = if trimmed == "~" || trimmed.starts_with("~/") {
        let home = home.filter(|h| !h.is_empty()).ok_or(ExternalRigError::HomeUnset)?;
        if trimmed == "~" {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(&trimmed[2..])
        }
    } else {
        let path = Path::new(trimmed);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            project_root.join(path)
        }
    };

    validate_external_rig_dir(candidate)
}

/// Validate a resolved external rig directory: exists, is a directory,
/// and is readable.
fn validate_external_rig_dir(
    candidate: PathBuf,
) -> Result<PathBuf, ExternalRigError> {
    if !candidate.exists() {
        return Err(ExternalRigError::NotFound(candidate));
    }
    if !candidate.is_dir() {
        return Err(ExternalRigError::NotADirectory(candidate));
    }
    std::fs::read_dir(&candidate).map_err(|e| {
        ExternalRigError::Unreadable(candidate.clone(), e.to_string())
    })?;
    Ok(candidate)
}

/// Resolve the **effective rig directory** for a local rig directory,
/// reading `$HOME` for `~/...` expansion.
///
/// `external` is the `external-rig` config value (`None` when the key
/// is absent). Returns the local rig directory unchanged for a
/// non-shim rig; for a shim rig, returns the resolved external
/// directory. The project root used for relative resolution is the
/// parent of `local_rig_dir` (matching the runtime-root derivation).
pub fn resolve_effective_rig_dir(
    local_rig_dir: &Path,
    external: Option<&str>,
) -> Result<PathBuf, ExternalRigError> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
    resolve_effective_rig_dir_in(local_rig_dir, external, home.as_deref())
}

/// Injectable-`$HOME` form of [`resolve_effective_rig_dir`].
pub fn resolve_effective_rig_dir_in(
    local_rig_dir: &Path,
    external: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, ExternalRigError> {
    let Some(raw) = external else {
        return Ok(local_rig_dir.to_path_buf());
    };
    let project_root = local_rig_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| local_rig_dir.to_path_buf());
    resolve_external_rig_in(raw, &project_root, home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Temp project layout: `<tmp>/project/rig` (local shim rig dir).
    struct Layout {
        project: tempfile::TempDir,
        local_rig: PathBuf,
    }

    fn layout() -> Layout {
        let project = tempfile::tempdir().unwrap();
        let local_rig = project.path().join("project").join("rig");
        fs::create_dir_all(&local_rig).unwrap();
        Layout {
            project,
            local_rig,
        }
    }

    // ── non-shim passthrough ──────────────────────────────────────────

    #[test]
    fn no_external_key_returns_local_rig_unchanged() {
        let l = layout();
        let resolved =
            resolve_effective_rig_dir(&l.local_rig, None).unwrap();
        assert_eq!(resolved, l.local_rig);
    }

    // ── resolution forms ──────────────────────────────────────────────

    #[test]
    fn absolute_path_is_used_as_is() {
        let l = layout();
        let external = l.project.path().join("shared/factory-rig");
        fs::create_dir_all(&external).unwrap();
        let resolved =
            resolve_effective_rig_dir(&l.local_rig, Some(&external.to_string_lossy()))
                .unwrap();
        assert_eq!(resolved, external);
    }

    #[test]
    fn relative_path_resolves_against_project_root() {
        let l = layout();
        // project root = <tmp>/project; relative "shared/factory-rig"
        let external = l.project.path().join("project/shared/factory-rig");
        fs::create_dir_all(&external).unwrap();
        let resolved = resolve_effective_rig_dir(
            &l.local_rig,
            Some("shared/factory-rig"),
        )
        .unwrap();
        assert_eq!(resolved, external);
    }

    #[test]
    fn tilde_path_expands_from_home() {
        let l = layout();
        let fake_home = l.project.path().join("fakehome");
        let external = fake_home.join("shared/factory-rig");
        fs::create_dir_all(&external).unwrap();
        let resolved = resolve_effective_rig_dir_in(
            &l.local_rig,
            Some("~/shared/factory-rig"),
            Some(fake_home.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(resolved, external);
    }

    #[test]
    fn bare_tilde_expands_to_home() {
        let fake_home = tempfile::tempdir().unwrap();
        let l = layout();
        let resolved = resolve_effective_rig_dir_in(
            &l.local_rig,
            Some("~"),
            Some(fake_home.path().to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(resolved, fake_home.path());
    }

    #[test]
    fn tilde_path_without_home_is_home_unset_error() {
        let l = layout();
        let err = resolve_effective_rig_dir_in(
            &l.local_rig,
            Some("~/somewhere"),
            None,
        )
        .unwrap_err();
        assert_eq!(err, ExternalRigError::HomeUnset);
    }

    #[test]
    fn empty_home_counts_as_unset() {
        let l = layout();
        let err = resolve_external_rig_in(
            "~/somewhere",
            l.project.path(),
            Some(""),
        )
        .unwrap_err();
        assert_eq!(err, ExternalRigError::HomeUnset);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let l = layout();
        let external = l.project.path().join("padded-rig");
        fs::create_dir_all(&external).unwrap();
        let resolved = resolve_effective_rig_dir(
            &l.local_rig,
            Some(&format!("  {}  ", external.display())),
        )
        .unwrap();
        assert_eq!(resolved, external);
    }

    // ── validation failures ───────────────────────────────────────────

    #[test]
    fn empty_value_is_an_error() {
        let l = layout();
        assert_eq!(
            resolve_effective_rig_dir(&l.local_rig, Some("")).unwrap_err(),
            ExternalRigError::Empty
        );
        assert_eq!(
            resolve_effective_rig_dir(&l.local_rig, Some("   "))
                .unwrap_err(),
            ExternalRigError::Empty
        );
    }

    #[test]
    fn missing_path_is_not_found_error() {
        let l = layout();
        let err = resolve_effective_rig_dir(
            &l.local_rig,
            Some("no/such/rig-anywhere"),
        )
        .unwrap_err();
        match err {
            ExternalRigError::NotFound(p) => assert!(p.ends_with("no/such/rig-anywhere")),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn file_path_is_not_a_directory_error() {
        let l = layout();
        let file = l.project.path().join("project/not-a-dir");
        fs::write(&file, "i am a file").unwrap();
        let err = resolve_effective_rig_dir(
            &l.local_rig,
            Some("not-a-dir"),
        )
        .unwrap_err();
        match err {
            ExternalRigError::NotADirectory(p) => assert_eq!(p, file),
            other => panic!("expected NotADirectory, got {other:?}"),
        }
    }

    #[test]
    fn error_messages_name_the_path() {
        let l = layout();
        let err = resolve_effective_rig_dir(
            &l.local_rig,
            Some("gone/factory-rig"),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("gone/factory-rig") && msg.contains("does not exist"),
            "message should name the path and the failure: {msg}"
        );
    }

    // ── config parsing ────────────────────────────────────────────────

    #[test]
    fn config_parses_external_rig_key() {
        let cfg: crate::domain::value_objects::RigAgentConfig =
            serde_yaml::from_str(
                "agent-adapter: pi-stdio\nexternal-rig: ../shared/factory-rig\n",
            )
            .unwrap();
        assert_eq!(
            cfg.external_rig.as_deref(),
            Some("../shared/factory-rig")
        );
    }

    #[test]
    fn config_without_external_rig_is_none() {
        let cfg: crate::domain::value_objects::RigAgentConfig =
            serde_yaml::from_str("agent-adapter: pi-stdio\n").unwrap();
        assert_eq!(cfg.external_rig, None);
    }
}
