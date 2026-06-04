use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::application::ports::{LoomRepository, PortError};
use crate::domain::entities::{Knot, KnotId, Loom, LoomId};
use crate::domain::knot_file::{self as knot_file_parser, KnotFile};

/// Filesystem-backed implementation of `LoomRepository`.
///
/// Scans a workspace directory for looms (subdirectories) and parses
/// `.md` knot definition files using `KnotFileParser` from the domain layer.
///
/// Also maintains an in-memory registry of saved looms for `get()`,
/// `list()`, and `save()` operations.
#[derive(Clone)]
pub struct FileSystemLoomRepository {
    /// In-memory registry of looms (populated by scan and save).
    looms: Arc<Mutex<HashMap<LoomId, Loom>>>,
}

impl Default for FileSystemLoomRepository {
    fn default() -> Self {
        Self {
            looms: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl FileSystemLoomRepository {
    /// Create a new empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a single loom in the internal map.
    fn register(&self, loom: Loom) {
        let mut map = self.looms.lock().unwrap();
        map.insert(loom.id.clone(), loom);
    }
}

impl LoomRepository for FileSystemLoomRepository {
    fn scan(&self, workspace: &Path) -> Result<Vec<Loom>, PortError> {
        let entries =
            fs::read_dir(workspace)
                .map_err(|e| PortError::WorkspaceScanFailed(e.to_string()))?;

        for entry_result in entries {
            let entry = entry_result
                .map_err(|e| PortError::WorkspaceScanFailed(e.to_string()))?;

            let file_type = entry
                .file_type()
                .map_err(|e| PortError::WorkspaceScanFailed(e.to_string()))?;

            if !file_type.is_dir() {
                continue;
            }

            let loom_dir = entry.path();
            let loom_name = loom_dir
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| {
                    PortError::WorkspaceScanFailed(
                        "invalid loom directory name".to_string(),
                    )
                })?
                .to_string();

            let loom_id = LoomId(loom_name.clone());

            // Canonicalise the loom directory to an absolute path.
            let canonical_loom_dir = fs::canonicalize(&loom_dir)
                .map_err(|e| {
                    PortError::WorkspaceScanFailed(format!(
                        "failed to canonicalise {}: {}",
                        loom_dir.display(),
                        e
                    ))
                })?;

            // Read .loom-config.yaml (falls back to defaults if absent/invalid).
            let config = read_loom_config(&canonical_loom_dir);

            // Resolve source_dir: config value → loom dir (default).
            let source_dir = resolve_config_path(
                &canonical_loom_dir,
                config.source_dir.as_ref(),
                &canonical_loom_dir,
            );

            // Resolve tie_off_dir: config value → <loom>/.knot-output (default).
            let default_tie_off_dir = canonical_loom_dir.join(".knot-output");
            let tie_off_dir = resolve_config_path(
                &canonical_loom_dir,
                config.tie_off_dir.as_ref(),
                &default_tie_off_dir,
            );

            // Parse .md knot definition files from the loom directory.
            // (Knot definitions always live in the loom dir alongside
            // `.loom-config.yaml` — `source_dir` points to where strands
            // come from, which may be external.)
            let knots = Self::scan_knot_files(&canonical_loom_dir)?;

            let loom = Loom {
                id: loom_id,
                source_dir,
                tie_off_dir,
                knots,
            };

            self.register(loom);
        }

        let looms_map = self.looms.lock().unwrap();
        let looms: Vec<Loom> = looms_map.values().cloned().collect();

        Ok(looms)
    }

    fn get(&self, id: &LoomId) -> Result<Option<Loom>, PortError> {
        let map = self.looms.lock().unwrap();
        Ok(map.get(id).cloned())
    }

    fn list(&self) -> Result<Vec<Loom>, PortError> {
        let map = self.looms.lock().unwrap();
        Ok(map.values().cloned().collect())
    }

    fn save(&self, loom: Loom) -> Result<(), PortError> {
        self.register(loom);
        Ok(())
    }
}

impl FileSystemLoomRepository {
    /// Scan a loom directory for `.md` knot definition files and parse them.
    ///
    /// Files that fail to parse are skipped with a warning log.
    fn scan_knot_files(
        loom_dir: &Path,
    ) -> Result<Vec<Knot>, PortError> {
        let entries = fs::read_dir(loom_dir)
            .map_err(|e| PortError::WorkspaceScanFailed(e.to_string()))?;

        let mut knots = Vec::new();

        for entry_result in entries {
            let entry = entry_result
                .map_err(|e| PortError::WorkspaceScanFailed(e.to_string()))?;

            let path = entry.path();

            // Skip directories and non-.md files.
            if path.is_dir() {
                continue;
            }
            match path.extension().and_then(|e| e.to_str()) {
                Some("md") => {}
                _ => continue,
            }

            let content =
                fs::read_to_string(&path).map_err(|e| {
                    PortError::WorkspaceScanFailed(format!(
                        "failed to read {}: {}",
                        path.display(),
                        e
                    ))
                })?;

            let parsed = knot_file_parser::parse(&content);
            match parsed {
                Ok(knot_file) => {
                    let knot = Self::knot_from_file(knot_file);
                    knots.push(knot);
                }
                Err(e) => {
                    eprintln!(
                        "WARNING: skipping invalid knot file {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        Ok(knots)
    }

    /// Convert a parsed `KnotFile` into a domain `Knot`.
    fn knot_from_file(file: KnotFile) -> Knot {
        Knot {
            id: KnotId(file.name.clone()),
            agent_config: file.agent_config,
            prompt_template: file.prompt_template,
        }
    }
}

/// Configuration loaded from `.loom-config.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
struct LoomConfig {
    source_dir: Option<String>,
    tie_off_dir: Option<String>,
}

/// Resolve a path value from `.loom-config.yaml`.
///
/// - If the value is `None`, return the `default_dir`.
/// - If the value is an absolute path, canonicalise it.
/// - If the value is a relative path, join it to `loom_dir` then canonicalise.
///
/// On canonicalisation failure, falls back to `default_dir` (already canonical).
fn resolve_config_path(
    loom_dir: &Path,
    value: Option<&String>,
    default_dir: &Path,
) -> PathBuf {
    let Some(val) = value else {
        return default_dir.to_path_buf();
    };

    let path = if Path::new(val).is_absolute() {
        PathBuf::from(val)
    } else {
        loom_dir.join(val)
    };

    fs::canonicalize(&path).unwrap_or_else(|_| {
        eprintln!(
            "WARNING: could not canonicalise config path '{}', \
             using default",
            path.display()
        );
        default_dir.to_path_buf()
    })
}

/// Read and parse `.loom-config.yaml` from a loom directory.
///
/// Returns `Ok(LoomConfig)` with defaults filled in, or logs a warning
/// and returns the default config on parse failure.
fn read_loom_config(loom_dir: &Path) -> LoomConfig {
    let config_path = loom_dir.join(".loom-config.yaml");

    // File does not exist — return defaults
    if !config_path.exists() {
        return LoomConfig::default();
    }

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "WARNING: could not read {}: {}, using defaults",
                config_path.display(),
                e
            );
            return LoomConfig::default();
        }
    };

    serde_yaml::from_str(&content).unwrap_or_else(|e| {
        eprintln!(
            "WARNING: malformed YAML in {}: {}, using defaults",
            config_path.display(),
            e
        );
        LoomConfig::default()
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const VALID_KNOT_CONTENT: &str = "---
name: review-knot
agent-config:
  goal: \"Review PRD goals for clarity\"
  provider: \"openai\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: |
    Review the goals section of this PRD.
---

# Review Knot

This knot reviews PRD goals.
";

    /// Write a knot definition file with the given name and content.
    fn create_knot_file(
        dir: &Path,
        name: &str,
        content: &str,
    ) -> std::io::Result<()> {
        fs::write(dir.join(format!("{name}.md")), content)
    }

    /// Write a `.loom-config.yaml` file with the given content.
    fn write_loom_config(
        dir: &Path,
        content: &str,
    ) -> std::io::Result<()> {
        fs::write(dir.join(".loom-config.yaml"), content)
    }

    #[test]
    fn scan_empty_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let repo = FileSystemLoomRepository::new();

        let result =
            repo.scan(workspace.path());

        assert!(result.is_ok());
        let looms = result.unwrap();
        assert!(
            looms.is_empty(),
            "empty workspace should return no looms"
        );
    }

    #[test]
    fn scan_workspace_with_one_loom() {
        let workspace = tempfile::tempdir().unwrap();

        // Create one loom directory with one valid knot file.
        let loom_dir = workspace.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(workspace.path());

        assert!(result.is_ok());
        let looms = result.unwrap();
        assert_eq!(
            looms.len(),
            1,
            "workspace with one loom directory should return one loom"
        );

        let loom = &looms[0];
        assert_eq!(loom.id, LoomId("my-loom".to_string()));
        assert_eq!(loom.source_dir, loom_dir);
        assert_eq!(loom.knots.len(), 1);
        assert_eq!(loom.knots[0].id, KnotId("review-knot".to_string()));
    }

    #[test]
    fn scan_workspace_with_multiple_looms() {
        let workspace = tempfile::tempdir().unwrap();

        // Loom 1.
        let loom1_dir = workspace.path().join("loom-a");
        fs::create_dir(&loom1_dir).unwrap();
        create_knot_file(&loom1_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // Loom 2.
        let loom2_dir = workspace.path().join("loom-b");
        fs::create_dir(&loom2_dir).unwrap();
        create_knot_file(&loom2_dir, "knot2", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(workspace.path());

        assert!(result.is_ok());
        let looms = result.unwrap();
        assert_eq!(
            looms.len(),
            2,
            "workspace with two loom directories should return two looms"
        );

        // Verify both looms are present.
        let ids: Vec<_> = looms.iter().map(|l| &l.id).collect();
        assert!(ids.contains(&&LoomId("loom-a".to_string())));
        assert!(ids.contains(&&LoomId("loom-b".to_string())));

        // Each loom has one knot.
        for loom in &looms {
            assert_eq!(loom.knots.len(), 1);
        }
    }

    #[test]
    fn scan_skips_invalid_knot_files() {
        let workspace = tempfile::tempdir().unwrap();

        let loom_dir = workspace.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        // One valid knot file.
        create_knot_file(&loom_dir, "valid", VALID_KNOT_CONTENT).unwrap();

        // One invalid knot file (malformed YAML frontmatter).
        let invalid_content = "---
broken: yaml: [
  unclosed
";
        create_knot_file(&loom_dir, "invalid", invalid_content).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(workspace.path());

        assert!(result.is_ok(), "scan should succeed even with invalid files");
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1);

        // The loom should contain only the valid knot.
        let loom = &looms[0];
        assert_eq!(loom.knots.len(), 1, "invalid knot should be skipped");
        assert_eq!(loom.knots[0].id, KnotId("review-knot".to_string()));
    }

    #[test]
    fn scan_parses_knot_definition_files() {
        let workspace = tempfile::tempdir().unwrap();

        let loom_dir = workspace.path().join("test-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "goals-review", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(workspace.path());

        assert!(result.is_ok());
        let looms = result.unwrap();
        let loom = &looms[0];
        let knot = &loom.knots[0];

        // Verify knot name parsed from frontmatter.
        assert_eq!(knot.id, KnotId("review-knot".to_string()));

        // Verify agent config parsed from frontmatter.
        assert_eq!(
            knot.agent_config.goal,
            "Review PRD goals for clarity"
        );

        // Verify prompt template parsed from frontmatter.
        assert_eq!(knot.prompt_template.input_bundling, "full-file");
        assert!(knot.prompt_template.instructions.contains("Review the goals"));
    }

    #[test]
    fn get_nonexistent_loom() {
        let repo = FileSystemLoomRepository::new();

        let result =
            repo.get(&LoomId("does-not-exist".to_string()));

        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "get for unknown ID should return Ok(None)"
        );
    }

    #[test]
    fn save_and_list_loom() {
        let repo = FileSystemLoomRepository::new();

        let loom = Loom {
            id: LoomId("saved-loom".to_string()),
            source_dir: std::path::PathBuf::from("src/prds"),
            tie_off_dir: std::path::PathBuf::from("output/prds"),
            knots: vec![],
        };

        let save_result = repo.save(loom.clone());
        assert!(save_result.is_ok());

        // list() should return the saved loom.
        let list_result = repo.list();
        assert!(list_result.is_ok());
        let listed = list_result.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, LoomId("saved-loom".to_string()));
        assert_eq!(listed[0].source_dir, std::path::PathBuf::from("src/prds"));

        // get() should also return the saved loom.
        let get_result = repo.get(&LoomId("saved-loom".to_string()));
        assert!(get_result.is_ok());
        let loom = get_result.unwrap();
        assert!(loom.is_some());
        assert_eq!(loom.unwrap().id, LoomId("saved-loom".to_string()));
    }

    #[test]
    fn scan_workspace_with_relative_path() {
        let temp_root = tempfile::tempdir().unwrap();

        // Create a workspace subdirectory.
        let ws_dir = temp_root.path().join("test-ws");
        fs::create_dir(&ws_dir).unwrap();

        // Create a loom directory inside the workspace.
        let loom_dir = ws_dir.join("relative-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // Build a relative path: "./test-ws" from the temp root.
        let rel_path =
            Path::new("./").join(ws_dir.file_name().unwrap());
        assert!(
            rel_path.to_string_lossy().contains("./"),
            "test path should contain ./ component"
        );

        // Change current dir to temp root so the relative path resolves.
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_root.path()).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(&rel_path);

        // Restore original directory (even on test failure).
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_ok(), "scan should succeed with relative path");
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1, "should find one loom");

        // Every loom's source_dir must be absolute.
        for loom in &looms {
            let source_str = loom.source_dir.to_string_lossy();
            assert!(
                loom.source_dir.is_absolute(),
                "source_dir should be absolute, got: {}",
                source_str
            );
            assert!(
                !source_str.contains("./"),
                "source_dir should not contain . component, got: {}",
                source_str
            );
            assert!(
                !source_str.contains("../"),
                "source_dir should not contain .. component, got: {}",
                source_str
            );
        }
    }

    #[test]
    fn scan_workspace_with_absolute_path() {
        let workspace = tempfile::tempdir().unwrap();

        // Create a loom directory inside the temp workspace.
        let loom_dir = workspace.path().join("abs-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let abs_path = workspace.path().to_path_buf();
        assert!(abs_path.is_absolute(), "test path should be absolute");

        let result = repo.scan(&abs_path);
        assert!(result.is_ok(), "scan should succeed with absolute path");
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1, "should find one loom");

        let loom = &looms[0];
        let source_str = loom.source_dir.to_string_lossy();

        // source_dir must be absolute.
        assert!(
            loom.source_dir.is_absolute(),
            "source_dir should be absolute, got: {}",
            source_str
        );

        // No double-slashes in the canonicalised path.
        assert!(
            !source_str.contains("//"),
            "source_dir should not contain double-slashes, got: {}",
            source_str
        );
    }

    // ── .loom-config.yaml Tests ─────────────────────────────────────────

    #[test]
    fn scan_uses_loom_config_source_dir() {
        let temp_root = tempfile::tempdir().unwrap();

        // External source directory outside the scanned workspace.
        let external_source = temp_root.path().join("external-source");
        fs::create_dir(&external_source).unwrap();

        // Workspace is a subdirectory (scan only sees loom inside it).
        let workspace = temp_root.path().join("workspace");
        fs::create_dir(&workspace).unwrap();

        // Loom directory (contains knot definitions and config).
        let loom_dir = workspace.join("config-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // .loom-config.yaml pointing source_dir to the external directory.
        write_loom_config(
            &loom_dir,
            "source_dir: ../../external-source",
        )
        .unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(&workspace);

        assert!(result.is_ok(), "scan should succeed with config");
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1, "should find one loom");

        let loom = &looms[0];
        // source_dir should resolve to the external directory.
        assert_eq!(
            loom.source_dir, external_source,
            "source_dir should be the external directory"
        );
        // source_dir must be absolute.
        assert!(loom.source_dir.is_absolute());
    }

    #[test]
    fn scan_uses_loom_config_tie_off_dir() {
        let temp_root = tempfile::tempdir().unwrap();

        // External tie-off directory outside the scanned workspace.
        let external_tie_off = temp_root.path().join("external-output");
        fs::create_dir(&external_tie_off).unwrap();

        // Workspace is a subdirectory.
        let workspace = temp_root.path().join("workspace");
        fs::create_dir(&workspace).unwrap();

        // Loom directory.
        let loom_dir = workspace.join("config-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // .loom-config.yaml pointing tie_off_dir to external directory.
        write_loom_config(
            &loom_dir,
            "tie_off_dir: ../../external-output",
        )
        .unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(&workspace);

        assert!(result.is_ok(), "scan should succeed with config");
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1, "should find one loom");

        let loom = &looms[0];
        assert_eq!(
            loom.tie_off_dir, external_tie_off,
            "tie_off_dir should be the external directory"
        );
        assert!(loom.tie_off_dir.is_absolute());
    }

    #[test]
    fn scan_fallback_defaults_without_config() {
        let workspace = tempfile::tempdir().unwrap();

        // Loom directory with NO .loom-config.yaml.
        let loom_dir = workspace.path().join("default-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(workspace.path());

        assert!(result.is_ok());
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1);

        let loom = &looms[0];
        // source_dir defaults to the loom directory.
        assert_eq!(
            loom.source_dir, loom_dir,
            "source_dir should default to loom directory"
        );
        // tie_off_dir defaults to <loom>/.knot-output.
        assert_eq!(
            loom.tie_off_dir,
            loom_dir.join(".knot-output"),
            "tie_off_dir should default to <loom>/.knot-output"
        );
    }

    #[test]
    fn scan_loom_config_absolute_paths() {
        let temp_root = tempfile::tempdir().unwrap();

        // External directories with absolute paths (outside scanned workspace).
        let abs_source = temp_root.path().join("abs-source");
        fs::create_dir(&abs_source).unwrap();
        let abs_tie_off = temp_root.path().join("abs-output");
        fs::create_dir(&abs_tie_off).unwrap();

        // Workspace is a subdirectory.
        let workspace = temp_root.path().join("workspace");
        fs::create_dir(&workspace).unwrap();

        // Loom directory.
        let loom_dir = workspace.join("abs-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // .loom-config.yaml with absolute paths.
        let config_content = format!(
            "source_dir: {}\ntie_off_dir: {}",
            abs_source.display(),
            abs_tie_off.display()
        );
        write_loom_config(&loom_dir, &config_content).unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(&workspace);

        assert!(result.is_ok(), "scan should succeed with absolute paths");
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1);

        let loom = &looms[0];
        assert_eq!(
            loom.source_dir, abs_source,
            "source_dir should use the absolute path"
        );
        assert_eq!(
            loom.tie_off_dir, abs_tie_off,
            "tie_off_dir should use the absolute path"
        );
    }

    #[test]
    fn scan_loom_config_malformed_yaml() {
        let workspace = tempfile::tempdir().unwrap();

        // Loom directory with a malformed .loom-config.yaml.
        let loom_dir = workspace.path().join("malformed-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // Invalid YAML content.
        write_loom_config(&loom_dir, "broken: yaml: [\n  unclosed").unwrap();

        let repo = FileSystemLoomRepository::new();
        let result = repo.scan(workspace.path());

        // Should succeed — falls back to defaults.
        assert!(
            result.is_ok(),
            "scan should succeed even with malformed config"
        );
        let looms = result.unwrap();
        assert_eq!(looms.len(), 1);

        let loom = &looms[0];
        // Falls back to defaults.
        assert_eq!(
            loom.source_dir, loom_dir,
            "source_dir should fall back to loom directory"
        );
        assert_eq!(
            loom.tie_off_dir,
            loom_dir.join(".knot-output"),
            "tie_off_dir should fall back to <loom>/.knot-output"
        );
    }

    #[test]
    fn read_loom_config_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = read_loom_config(dir.path());

        assert!(config.source_dir.is_none());
        assert!(config.tie_off_dir.is_none());
    }

    #[test]
    fn read_loom_config_parses_valid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(".loom-config.yaml"),
            "source_dir: ../app\ntie_off_dir: ../output",
        )
        .unwrap();

        let config = read_loom_config(dir.path());

        assert_eq!(
            config.source_dir,
            Some("../app".to_string()),
            "source_dir should be parsed"
        );
        assert_eq!(
            config.tie_off_dir,
            Some("../output".to_string()),
            "tie_off_dir should be parsed"
        );
    }

    #[test]
    fn resolve_config_path_relative_joins_to_loom_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let loom_dir = workspace.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        let external = workspace.path().join("external");
        fs::create_dir(&external).unwrap();

        let default_dir = loom_dir.clone();
        let resolved = resolve_config_path(
            &loom_dir,
            Some(&"../external".to_string()),
            &default_dir,
        );

        assert_eq!(
            resolved, external,
            "relative path should resolve against loom dir"
        );
        assert!(resolved.is_absolute());
    }

    #[test]
    fn resolve_config_path_none_returns_default() {
        let workspace = tempfile::tempdir().unwrap();
        let loom_dir = workspace.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        let default_dir = fs::canonicalize(&loom_dir).unwrap();
        let resolved = resolve_config_path(&loom_dir, None, &default_dir);

        assert_eq!(
            resolved, default_dir,
            "None value should return default_dir"
        );
    }

    #[test]
    fn resolve_config_path_absolute_uses_as_is() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("target");
        fs::create_dir(&target).unwrap();

        let loom_dir = workspace.path().join("loom");
        fs::create_dir(&loom_dir).unwrap();

        let default_dir = fs::canonicalize(&loom_dir).unwrap();
        let resolved = resolve_config_path(
            &loom_dir,
            Some(&target.to_string_lossy().to_string()),
            &default_dir,
        );

        assert_eq!(
            resolved, target,
            "absolute path should resolve to the target"
        );
    }
}

