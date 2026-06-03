use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
            let source_dir = loom_dir.clone();

            // Parse .md knot definition files from the loom directory.
            let knots = Self::scan_knot_files(&source_dir)?;

            let loom = Loom {
                id: loom_id,
                source_dir: source_dir.clone(),
                tie_off_dir: source_dir.join(".knot-output"),
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const VALID_KNOT_CONTENT: &str = "---
name: review-knot
agent-config:
  goal: \"Review PRD goals for clarity\"
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
}
