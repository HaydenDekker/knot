use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::application::ports::{LoomRepository, PortError};
use crate::domain::entities::{Knot, KnotId, Loom, LoomId};
use crate::domain::knot_file as knot_file_parser;

/// Filesystem-backed implementation of `LoomRepository`.
///
/// Scans a rig directory for looms (subdirectories) and parses
/// `.md` knot definition files using `KnotFileParser` from the domain layer.
///
/// Each knot defines its input source in its frontmatter via `strand-dir` (required).
/// For filesystem sources, relative paths are resolved against the project root.
/// Tie-off paths are statically derived from loom ID and knot name.
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
    fn scan_knot_files(
        &self,
        loom_dir: &Path,
    ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
        Self::scan_knot_files(loom_dir)
    }

    fn scan(&self, rig: &Path) -> Result<(Vec<Loom>, Vec<String>), PortError> {
        // Canonicalise the rig directory, then use its parent as the
        // base for resolving per-knot paths (project root).
        let canonical_rig = fs::canonicalize(rig)
            .map_err(|e| {
                PortError::RigScanFailed(format!(
                    "failed to canonicalise {}: {}",
                    rig.display(),
                    e
                ))
            })?;
        let project_root = canonical_rig
            .parent()
            .unwrap_or(&canonical_rig);

        let entries =
            fs::read_dir(rig)
                .map_err(|e| PortError::RigScanFailed(e.to_string()))?;

        let mut all_warnings = Vec::new();

        for entry_result in entries {
            let entry = entry_result
                .map_err(|e| PortError::RigScanFailed(e.to_string()))?;

            let file_type = entry
                .file_type()
                .map_err(|e| PortError::RigScanFailed(e.to_string()))?;

            if !file_type.is_dir() {
                continue;
            }

            let loom_dir = entry.path();
            let loom_name = loom_dir
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| {
                    PortError::RigScanFailed(
                        "invalid loom directory name".to_string(),
                    )
                })?
                .to_string();

            // Only discover directories ending in `-loom` (naming convention).
            // This prevents phantom looms from state directories like
            // `<rig>/<id>/` created by LoomLogPort::open().
            if !loom_name.ends_with("-loom") {
                continue;
            }

            let loom_id = LoomId(loom_name.clone());

            // Canonicalise the loom directory to an absolute path.
            let canonical_loom_dir = fs::canonicalize(&loom_dir)
                .map_err(|e| {
                    PortError::RigScanFailed(format!(
                        "failed to canonicalise {}: {}",
                        loom_dir.display(),
                        e
                    ))
                })?;

            // Parse .md knot definition files from the loom directory,
            // capturing parse warnings per-loom.
            let (mut knots, mut warnings) = Self::scan_knot_files(&canonical_loom_dir)?;
            all_warnings.append(&mut warnings);

            // Resolve per-knot paths relative to the project root
            // (parent of the rig directory).
            // For Filesystem strand sources, resolve the path to absolute.
            // For EventUri sources, no filesystem path to resolve.
            knots = knots.into_iter().map(|knot| {
                let resolved = knot.strand_source.path()
                    .map(|p| Self::resolve_path(project_root, &p.to_path_buf()))
                    .unwrap_or_default();
                knot.with_strand_dir(resolved)
            }).collect();

            let loom = Loom {
                id: loom_id,
                knots,
            };

            self.register(loom);
        }

        let looms_map = self.looms.lock().unwrap();
        let looms: Vec<Loom> = looms_map.values().cloned().collect();

        Ok((looms, all_warnings))
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
    /// Scan a directory for `.md` knot definition files and parse them.
    ///
    /// Files that fail to parse are skipped. Unknown YAML properties
    /// in valid files produce warnings (see below).
    ///
    /// # Arguments
    ///
    /// * `knot_dir` - Directory to scan for `.md` knot files.
    ///
    /// # Returns
    ///
    /// A tuple of:
    /// - Parsed `Knot` instances with unresolved paths (caller must resolve
    ///   `strand_source` filesystem paths relative to the project root).
    /// - A vector of warning strings for unknown YAML properties in
    ///   the parsed knot frontmatter.
    pub fn scan_knot_files(
        knot_dir: &Path,
    ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
        let entries = fs::read_dir(knot_dir)
            .map_err(|e| PortError::RigScanFailed(e.to_string()))?;

        let mut knots = Vec::new();
        let mut warnings = Vec::new();

        for entry_result in entries {
            let entry = entry_result
                .map_err(|e| PortError::RigScanFailed(e.to_string()))?;

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
                    PortError::RigScanFailed(format!(
                        "failed to read {}: {}",
                        path.display(),
                        e
                    ))
                })?;

            let parsed = knot_file_parser::parse(&content);
            match parsed {
                Ok((knot_file, file_warnings)) => {
                    warnings.extend(file_warnings);
                    let knot = crate::domain::entities::Knot::from_file(knot_file);
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

        Ok((knots, warnings))
    }

    /// Resolve a path value relative to a base directory.
    ///
    /// - If the value starts with `~`, expand it to the user's home directory
    ///   first.
    /// - If the value is an absolute path, canonicalise it.
    /// - If the value is a relative path, join it to `loom_dir`, then
    ///   canonicalise.
    ///
    /// On canonicalisation failure (e.g. directory does not exist yet),
    /// returns the path normalised by resolving `.` and `..` components.
    pub fn resolve_path(loom_dir: &Path, value: &PathBuf) -> PathBuf {
        let path = if let Some(home) = std::env::var_os("HOME") {
            // Expand `~` prefix to user's home directory.
            let value_str = value.to_string_lossy();
            if let Some(rest) = value_str.strip_prefix('~') {
                if rest.is_empty() || rest.starts_with('/') {
                    // `~` or `~/path` — expand to $HOME
                    let home_path = PathBuf::from(home);
                    if rest.is_empty() {
                        home_path
                    } else {
                        // rest starts with `/` — append the remainder
                        // to home_path (trim the leading `/`).
                        home_path.join(rest.trim_start_matches('/'))
                    }
                } else {
                    // Not a home-path prefix (e.g. `~tilde` without `/`)
                    value.clone()
                }
            } else {
                value.clone()
            }
        } else {
            value.clone()
        };

        let path = if path.is_absolute() {
            path
        } else {
            loom_dir.join(&path)
        };

        fs::canonicalize(&path).unwrap_or_else(|_| {
            // Directory might not exist yet; normalise manually by
            // resolving `.` and `..` components. The base (loom_dir)
            // is already canonical, so we can safely pop `..`.
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
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const VALID_KNOT_CONTENT: &str = "---
name: review-knot
agent-profile-ref: fast
strand-dir: \"../external-source\"
---

Review the goals section of this PRD.
";

    const KNOT_WITH_DIRS_CONTENT: &str = "---
name: custom-dirs-knot
agent-profile-ref: fast
strand-dir: \"../external-source\"
---

Review with custom dirs
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
    fn scan_empty_rig() {
        let rig = tempfile::tempdir().unwrap();
        let repo = FileSystemLoomRepository::new();

        let (looms, warnings) =
            repo.scan(rig.path()).unwrap();

        assert!(
            looms.is_empty(),
            "empty rig should return no looms"
        );
        assert!(
            warnings.is_empty(),
            "empty rig should have no warnings"
        );
    }

    #[test]
    fn scan_rig_with_one_loom() {
        let rig = tempfile::tempdir().unwrap();

        // Create one loom directory with one valid knot file.
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(
            looms.len(),
            1,
            "rig with one loom directory should return one loom"
        );
        assert!(
            warnings.is_empty(),
            "no warnings expected for valid knot, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.id, LoomId("my-loom".to_string()));
        assert_eq!(loom.knots.len(), 1);
        assert_eq!(loom.knots[0].id, KnotId("review-knot".to_string()));

        // Knot has required dirs (resolved to absolute paths).
        let knot = &loom.knots[0];
        assert!(knot.strand_source.path().map(|p| p.is_absolute()).unwrap_or(false));
    }

    #[test]
    fn scan_rig_with_multiple_looms() {
        let rig = tempfile::tempdir().unwrap();

        // Loom 1.
        let loom1_dir = rig.path().join("loom-a-loom");
        fs::create_dir(&loom1_dir).unwrap();
        create_knot_file(&loom1_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // Loom 2.
        let loom2_dir = rig.path().join("loom-b-loom");
        fs::create_dir(&loom2_dir).unwrap();
        create_knot_file(&loom2_dir, "knot2", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(
            looms.len(),
            2,
            "rig with two loom directories should return two looms"
        );
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        // Verify both looms are present.
        let ids: Vec<_> = looms.iter().map(|l| &l.id).collect();
        assert!(ids.contains(&&LoomId("loom-a-loom".to_string())));
        assert!(ids.contains(&&LoomId("loom-b-loom".to_string())));

        // Each loom has one knot.
        for loom in &looms {
            assert_eq!(loom.knots.len(), 1);
        }
    }

    #[test]
    fn scan_skips_invalid_knot_files() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("my-loom");
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
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);

        // The loom should contain only the valid knot.
        let loom = &looms[0];
        assert_eq!(loom.knots.len(), 1, "invalid knot should be skipped");
        assert_eq!(loom.knots[0].id, KnotId("review-knot".to_string()));
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );
    }

    #[test]
    fn scan_parses_knot_definition_files() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("test-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "goals-review", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);
        let loom = &looms[0];
        let knot = &loom.knots[0];

        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        // Verify knot name parsed from frontmatter.
        assert_eq!(knot.id, KnotId("review-knot".to_string()));

        // Verify profile ref parsed from frontmatter.
        assert_eq!(knot.agent_profile_ref, "fast");

        // Verify prompt template parsed from frontmatter.
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

        // get() should also return the saved loom.
        let get_result = repo.get(&LoomId("saved-loom".to_string()));
        assert!(get_result.is_ok());
        let loom = get_result.unwrap();
        assert!(loom.is_some());
        assert_eq!(loom.unwrap().id, LoomId("saved-loom".to_string()));
    }

    #[test]
    fn scan_rig_with_relative_path() {
        let temp_root = tempfile::tempdir().unwrap();

        // Create a rig subdirectory.
        let ws_dir = temp_root.path().join("test-ws");
        fs::create_dir(&ws_dir).unwrap();

        // Create a loom directory inside the rig.
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
        let (looms, warnings) = repo.scan(&rel_path).unwrap();

        // Restore original directory (even on test failure).
        std::env::set_current_dir(&original_dir).unwrap();

        assert_eq!(looms.len(), 1, "should find one loom");
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        // Every loom's knots have absolute paths.
        for loom in &looms {
            for knot in &loom.knots {
                let path = knot.strand_source.path().expect("knot should have filesystem path");
                assert!(
                    path.is_absolute(),
                    "strand_source path should be absolute, got: {}",
                    path.display()
                );
                assert!(
                    !path.to_string_lossy().contains("./"),
                    "strand_source path should not contain . component, got: {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn scan_rig_with_absolute_path() {
        let rig = tempfile::tempdir().unwrap();

        // Create a loom directory inside the temp rig.
        let loom_dir = rig.path().join("abs-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let abs_path = rig.path().to_path_buf();
        assert!(abs_path.is_absolute(), "test path should be absolute");

        let (looms, warnings) = repo.scan(&abs_path).unwrap();
        assert_eq!(looms.len(), 1, "should find one loom");
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        let path = loom.knots[0].strand_source.path().expect("knot should have filesystem path");
        let strand_str = path.to_string_lossy();

        // strand_source path must be absolute.
        assert!(
            path.is_absolute(),
            "strand_source path should be absolute, got: {}",
            strand_str
        );

        // No double-slashes in the canonicalised path.
        assert!(
            !strand_str.contains("//"),
            "strand_source path should not contain double-slashes, got: {}",
            strand_str
        );
    }

    // ── Per-knot directory tests ──────────────────────────────────────

    #[test]
    fn scan_parses_per_knot_source_and_tieoff_dirs() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("per-knot-dirs-loom");
        fs::create_dir(&loom_dir).unwrap();

        // One knot with custom directories.
        create_knot_file(
            &loom_dir,
            "custom-knot",
            KNOT_WITH_DIRS_CONTENT,
        )
        .unwrap();

        // One knot without custom directories.
        create_knot_file(&loom_dir, "default-knot", VALID_KNOT_CONTENT)
            .unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.knots.len(), 2, "loom should have 2 knots");

        // Find knots by name.
        let custom_knot = loom
            .knots
            .iter()
            .find(|k| k.id == KnotId("custom-dirs-knot".to_string()))
            .expect("custom-dirs-knot should exist");
        let default_knot = loom
            .knots
            .iter()
            .find(|k| k.id == KnotId("review-knot".to_string()))
            .expect("review-knot should exist");

        // Custom knot has required directories (resolved to absolute).
        assert!(
            custom_knot.strand_source.path().map(|p| p.is_absolute()).unwrap_or(false),
            "custom knot strand_source path should be absolute"
        );

        // Default knot also has required directories (from frontmatter).
        assert!(
            default_knot.strand_source.path().map(|p| p.is_absolute()).unwrap_or(false),
            "default knot strand_source path should be absolute"
        );
    }

    #[test]
    fn scan_per_knot_source_dir_resolved_to_external() {
        let temp_root = tempfile::tempdir().unwrap();

        // External source directory outside the scanned rig.
        let external_source = temp_root.path().join("external-source");
        fs::create_dir(&external_source).unwrap();

        // Rig is a subdirectory (scan only sees loom inside it).
        let rig = temp_root.path().join("rig");
        fs::create_dir(&rig).unwrap();

        // Loom directory (contains knot definitions).
        let loom_dir = rig.join("config-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot with per-knot strand-dir.
        // Paths resolve relative to project root (rig's parent = temp_root),
        // so we use simple names that are siblings of rig/.
        let knot_content = r#"---
name: custom-dirs-knot
agent-profile-ref: fast
strand-dir: "external-source"
---

Review with custom dirs
"#;
        create_knot_file(&loom_dir, "custom-knot", knot_content).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(&rig).unwrap();

        assert_eq!(looms.len(), 1, "should find one loom");
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.knots.len(), 1);

        let knot = &loom.knots[0];
        // strand_source resolves relative to project root (rig's parent).
        // "external-source" from temp_root → temp_root/external-source.
        let path = knot.strand_source.path().expect("knot should have filesystem path");
        assert_eq!(
            path, &external_source,
            "knot strand_source path should resolve relative to project root"
        );
    }

    #[test]
    fn scan_multiple_knots_different_source_dirs() {
        let temp_root = tempfile::tempdir().unwrap();

        // Two external source directories.
        let source_a = temp_root.path().join("source-a");
        fs::create_dir(&source_a).unwrap();
        let source_b = temp_root.path().join("source-b");
        fs::create_dir(&source_b).unwrap();

        // Rig and loom.
        let rig = temp_root.path().join("rig");
        fs::create_dir(&rig).unwrap();
        let loom_dir = rig.join("multi-source-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot A with its own source dir.
        let knot_a_content = format!(
            "---\nname: knot-a\nagent-profile-ref: fast\nstrand-dir: \"{}\"\n---\n\nReview A\n",
            source_a.display(),
        );
        create_knot_file(&loom_dir, "knot-a", &knot_a_content).unwrap();

        // Knot B with its own source dir.
        let knot_b_content = format!(
            "---\nname: knot-b\nagent-profile-ref: fast\nstrand-dir: \"{}\"\n---\n\nReview B\n",
            source_b.display(),
        );
        create_knot_file(&loom_dir, "knot-b", &knot_b_content).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(&rig).unwrap();

        assert_eq!(looms.len(), 1);
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.knots.len(), 2, "loom should have 2 knots");

        let knot_a = loom
            .knots
            .iter()
            .find(|k| k.id == KnotId("knot-a".to_string()))
            .expect("knot-a should exist");
        let knot_b = loom
            .knots
            .iter()
            .find(|k| k.id == KnotId("knot-b".to_string()))
            .expect("knot-b should exist");

        // Each knot has its own source directory.
        let path_a = knot_a.strand_source.path().expect("knot-a should have filesystem path");
        let path_b = knot_b.strand_source.path().expect("knot-b should have filesystem path");
        assert_eq!(
            path_a, &source_a,
            "knot-a should have source-a as strand_source path"
        );
        assert_eq!(
            path_b, &source_b,
            "knot-b should have source-b as strand_source path"
        );

        // They should be different.
        assert_ne!(
            path_a, path_b,
            "knots should have different strand directories"
        );
    }

    #[test]
    fn scan_knot_gets_required_dirs() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.id, LoomId("my-loom".to_string()));

        // Knot has required dirs (resolved to absolute paths).
        let knot = &loom.knots[0];
        let path = knot.strand_source.path().expect("knot should have filesystem path");
        assert!(path.is_absolute(), "strand_source path should be absolute");
    }

    #[test]
    fn resolve_path_relative_joins_to_loom_dir() {
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        let external = rig.path().join("external");
        fs::create_dir(&external).unwrap();

        let resolved = FileSystemLoomRepository::resolve_path(
            &loom_dir,
            &PathBuf::from("../external"),
        );

        assert_eq!(
            resolved, external,
            "relative path should resolve against loom dir"
        );
        assert!(resolved.is_absolute());
    }

    #[test]
    fn resolve_path_absolute_uses_as_is() {
        let rig = tempfile::tempdir().unwrap();
        let target = rig.path().join("target");
        fs::create_dir(&target).unwrap();

        let loom_dir = rig.path().join("loom");
        fs::create_dir(&loom_dir).unwrap();

        let resolved = FileSystemLoomRepository::resolve_path(
            &loom_dir,
            &target,
        );

        assert_eq!(
            resolved, target,
            "absolute path should resolve to the target"
        );
    }

    #[test]
    fn resolve_path_nonexistent_joins_without_canonicalise() {
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Path does not exist.
        let nonexistent = rig.path().join("nonexistent-dir");
        assert!(!nonexistent.exists());

        let resolved = FileSystemLoomRepository::resolve_path(
            &loom_dir,
            &PathBuf::from("../nonexistent-dir"),
        );

        assert_eq!(
            resolved, nonexistent,
            "nonexistent relative path should still join correctly"
        );
        assert!(resolved.is_absolute());
    }

    // ── Phase 2: -loom naming convention tests ─────────────────────────

    #[test]
    fn scan_skips_non_loom_directories() {
        let rig = tempfile::tempdir().unwrap();

        // Create a valid loom directory (ends in -loom).
        let loom_dir = rig.path().join("valid-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // Create a non-loom directory (e.g. tie-offs directory).
        let tieoffs_dir = rig.path().join("tie-offs");
        fs::create_dir(&tieoffs_dir).unwrap();
        create_knot_file(&tieoffs_dir, "knot2", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(
            looms.len(),
            1,
            "only -loom directories should be discovered"
        );
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );
        assert_eq!(
            looms[0].id,
            LoomId("valid-loom".to_string()),
            "should only find the -loom directory"
        );
    }

    #[test]
    fn scan_requires_strand_dir() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("required-dirs-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot without strand-dir — should be skipped.
        let no_strand_content = "---
name: no-strand-knot
agent-profile-ref: fast
---

Review
";
        create_knot_file(&loom_dir, "no-strand", no_strand_content).unwrap();

        // Knot without tie-off-dir — valid (tie-off-dir is no longer accepted).
        let no_tieoff_content = "---
name: no-tieoff-knot
agent-profile-ref: fast
strand-dir: \"../input\"
---

Review
";
        create_knot_file(&loom_dir, "no-tieoff", no_tieoff_content).unwrap();

        // Valid knot with strand-dir.
        create_knot_file(&loom_dir, "valid", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);

        // Two valid knots (no-tieoff + valid), no-strand is skipped.
        let loom = &looms[0];
        assert_eq!(
            loom.knots.len(),
            2,
            "knots missing strand-dir should be skipped, others accepted"
        );
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );
        let knot_names: Vec<_> = loom.knots.iter().map(|k| k.id.0.clone()).collect();
        assert!(knot_names.contains(&"no-tieoff-knot".to_string()));
        assert!(knot_names.contains(&"review-knot".to_string()));
    }

    #[test]
    fn scan_ignores_loom_log_directory() {
        let rig = tempfile::tempdir().unwrap();

        // Create a valid loom directory.
        let valid_loom_dir = rig.path().join("active-loom");
        fs::create_dir(&valid_loom_dir).unwrap();
        create_knot_file(&valid_loom_dir, "knot1", VALID_KNOT_CONTENT).unwrap();

        // Simulate a state directory created by LoomLogPort::open().
        // This would be <rig>/<some-id>/ (no -loom suffix).
        let state_dir = rig.path().join("some-loom-id");
        fs::create_dir(&state_dir).unwrap();
        // It has a .loom-log file (state file, not a loom).
        fs::write(state_dir.join(".loom-log"), "some log data").unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(
            looms.len(),
            1,
            "state directory should not be discovered as a loom"
        );
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );
        assert_eq!(
            looms[0].id,
            LoomId("active-loom".to_string()),
            "should only find the -loom directory"
        );
    }

    // ── Phase 5: strand_source construction tests (EventUri + Filesystem) ──

    /// Scan a loom that contains an EventUri knot — `strand-dir` is an
    /// event URI and `scan_knot_files` should parse it into
    /// `StrandSource::EventUri` with correct producer_knot and event_id.
    #[test]
    fn scan_loom_with_event_uri_knot() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("event-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot with event URI as strand-dir.
        let knot_content = r#"---
name: plan-validator
agent-profile-ref: fast
strand-dir: "event:plan-creator:PlanCreated"
event-description: When a plan is created
---

Validate the plan.
"#;
        fs::write(loom_dir.join("plan-validator.md"), knot_content).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1, "should find one loom");
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.id, LoomId("event-loom".to_string()));
        assert_eq!(loom.knots.len(), 1);

        let knot = &loom.knots[0];
        assert_eq!(knot.id, KnotId("plan-validator".to_string()));

        // Verify the strand_source is an EventUri with correct fields.
        match &knot.strand_source {
            crate::domain::value_objects::StrandSource::EventUri {
                producer_knot,
                event_id,
            } => {
                assert_eq!(
                    producer_knot, "plan-creator",
                    "producer_knot should be 'plan-creator'"
                );
                assert_eq!(
                    event_id, "PlanCreated",
                    "event_id should be 'PlanCreated'"
                );
            }
            crate::domain::value_objects::StrandSource::Filesystem(path) => {
                panic!(
                    "expected EventUri, got Filesystem({:?})",
                    path
                );
            }
        }

        // Event description should be preserved.
        assert_eq!(
            knot.event_description,
            Some("When a plan is created".to_string())
        );

        // EventUri knots have no filesystem path.
        assert!(
            knot.strand_source.path().is_none(),
            "EventUri should have no filesystem path"
        );

        // is_event should return true.
        assert!(
            knot.strand_source.is_event(),
            "EventUri knot should be_event = true"
        );
    }

    /// Scan a loom that contains a Filesystem knot — `strand-dir` is a plain
    /// path and `scan_knot_files` should parse it into
    /// `StrandSource::Filesystem` with the path resolved to absolute.
    #[test]
    fn scan_loom_with_filesystem_knot() {
        let rig = tempfile::tempdir().unwrap();

        // Create an actual strand directory so the path resolves.
        let strand_dir = rig.path().join("strands");
        fs::create_dir(&strand_dir).unwrap();

        let loom_dir = rig.path().join("fs-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot with plain path as strand-dir.
        let knot_content = format!(
            r#"---
name: review-knot
agent-profile-ref: fast
strand-dir: "{}"
---

Review the document.
"#,
            strand_dir.display()
        );
        fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1, "should find one loom");
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(loom.id, LoomId("fs-loom".to_string()));
        assert_eq!(loom.knots.len(), 1);

        let knot = &loom.knots[0];
        assert_eq!(knot.id, KnotId("review-knot".to_string()));

        // Verify the strand_source is a Filesystem with resolved path.
        match &knot.strand_source {
            crate::domain::value_objects::StrandSource::Filesystem(path) => {
                assert!(
                    path.is_absolute(),
                    "Filesystem path should be resolved to absolute, got: {:?}",
                    path
                );
                assert!(
                    path.exists(),
                    "Filesystem path should exist: {:?}",
                    path
                );
            }
            crate::domain::value_objects::StrandSource::EventUri { .. } => {
                panic!("expected Filesystem, got EventUri");
            }
        }

        // event_description should be None for a plain Filesystem knot.
        assert!(
            knot.event_description.is_none(),
            "Filesystem knot should have no event_description"
        );

        // is_event should return false.
        assert!(
            !knot.strand_source.is_event(),
            "Filesystem knot should have is_event = false"
        );
    }

    /// Scan a loom that contains both an EventUri knot and a Filesystem
    /// knot — both should be parsed and stored correctly.
    #[test]
    fn scan_loom_with_mixed_event_uri_and_filesystem_knots() {
        let rig = tempfile::tempdir().unwrap();

        // Create an actual strand directory for the Filesystem knot.
        let strand_dir = rig.path().join("strands");
        fs::create_dir(&strand_dir).unwrap();

        let loom_dir = rig.path().join("mixed-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Filesystem knot.
        let fs_knot_content = format!(
            r#"---
name: reviewer
agent-profile-ref: fast
strand-dir: "{}"
---

Review the document.
"#,
            strand_dir.display()
        );
        fs::write(loom_dir.join("reviewer.md"), fs_knot_content).unwrap();

        // EventUri knot.
        let event_knot_content = r#"---
name: plan-validator
agent-profile-ref: fast
strand-dir: "event:plan-creator:PlanCreated"
event-description: When a plan is created
---

Validate the plan.
"#;
        fs::write(
            loom_dir.join("plan-validator.md"),
            event_knot_content,
        )
        .unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1, "should find one loom");
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {:?}",
            warnings
        );

        let loom = &looms[0];
        assert_eq!(
            loom.knots.len(),
            2,
            "loom should have both knots"
        );

        // Find each knot by name.
        let reviewer_knot = loom
            .knots
            .iter()
            .find(|k| k.id == KnotId("reviewer".to_string()))
            .expect("reviewer knot should exist");
        let plan_validator_knot = loom
            .knots
            .iter()
            .find(|k| k.id == KnotId("plan-validator".to_string()))
            .expect("plan-validator knot should exist");

        // Filesystem knot should have a resolved path.
        match &reviewer_knot.strand_source {
            crate::domain::value_objects::StrandSource::Filesystem(path) => {
                assert!(path.is_absolute());
                assert!(path.exists());
            }
            _ => panic!(
                "reviewer should be Filesystem, got {:?}",
                reviewer_knot.strand_source
            ),
        }

        // EventUri knot should have correct event fields.
        match &plan_validator_knot.strand_source {
            crate::domain::value_objects::StrandSource::EventUri {
                producer_knot,
                event_id,
            } => {
                assert_eq!(producer_knot, "plan-creator");
                assert_eq!(event_id, "PlanCreated");
            }
            _ => panic!(
                "plan-validator should be EventUri, got {:?}",
                plan_validator_knot.strand_source
            ),
        }
    }

    // ── Phase 3: warning propagation tests ─────────────────────────────

    #[test]
    fn scan_returns_warnings_for_unknown_properties() {
        let rig = tempfile::tempdir().unwrap();

        let loom_dir = rig.path().join("warning-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot with an unknown YAML property.
        let knot_with_unknown = r#"---
name: legacy-knot
agent-profile-ref: fast
strand-dir: "strands"
tie-off-dir: "old-output"
---

Review
"#;
        create_knot_file(&loom_dir, "legacy", knot_with_unknown).unwrap();

        // Knot with clean frontmatter.
        create_knot_file(&loom_dir, "clean", VALID_KNOT_CONTENT).unwrap();

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);
        let loom = &looms[0];
        assert_eq!(loom.knots.len(), 2);

        // Should have exactly one warning (from the legacy knot).
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("tie-off-dir"),
            "warning should mention 'tie-off-dir', got: {}",
            warnings[0]
        );
    }

    // ── Tilde expansion tests ──────────────────────────────────────────

    #[test]
    fn resolve_path_expands_tilde_to_home() {
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        let home = std::env::var("HOME").expect("HOME should be set");
        let resolved =
            FileSystemLoomRepository::resolve_path(
                &loom_dir,
                &PathBuf::from("~/.agents/skills"),
            );

        // Should expand ~ to $HOME, not join to loom_dir.
        assert!(
            resolved.is_absolute(),
            "tilde-expanded path should be absolute, got: {}",
            resolved.display()
        );
        assert!(
            resolved.starts_with(&home),
            "path should start with home dir '{}', got: {}",
            home,
            resolved.display()
        );
        assert!(
            resolved.to_string_lossy().contains(".agents/skills"),
            "path should contain .agents/skills, got: {}",
            resolved.display()
        );
    }

    #[test]
    fn resolve_path_expands_bare_tilde() {
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        let home = std::env::var("HOME").expect("HOME should be set");
        let resolved =
            FileSystemLoomRepository::resolve_path(&loom_dir, &PathBuf::from("~"));

        assert!(
            resolved.is_absolute(),
            "bare tilde should expand to absolute home path"
        );
        assert_eq!(
            resolved.to_string_lossy(),
            home,
            "bare tilde should resolve to $HOME"
        );
    }

    #[test]
    fn resolve_path_tilde_not_at_start_is_relative() {
        // A path like "foo/~" is NOT a home-path prefix — the
        // tilde is not at the start. Should be treated as relative.
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        let resolved =
            FileSystemLoomRepository::resolve_path(
                &loom_dir,
                &PathBuf::from("foo/~"),
            );

        // Should be joined to loom_dir (relative), not expanded.
        assert!(
            resolved.to_string_lossy().contains("foo/~"),
            "tilde not at start should not be expanded, got: {}",
            resolved.display()
        );
    }
}
