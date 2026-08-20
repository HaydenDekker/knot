//! Filesystem-backed implementation of `ModelRegistryPort`.
//!
//! Reads `{rig_dir}/models.yml` **fresh on every `load()` call** — no
//! caching. Editing the registry swaps the model behind an alias on the
//! next strand processed, without a restart (the profile lifetime).
//!
//! Failure modes degrade to an empty registry (with a warning) so the
//! registry can never block strand processing: direct-spec profiles are
//! unaffected by an empty registry, and alias profiles fail with a
//! clear `ModelRefNotFound` error instead of an IO error.

use std::fs;
use std::path::PathBuf;

use crate::application::ports::{ModelRegistryPort, PortError};
use crate::domain::value_objects::ModelRegistry;

/// Filesystem-backed model registry reading `{rig_dir}/models.yml`.
#[derive(Clone)]
pub struct FileSystemModelRegistry {
    /// Rig directory (holds reusable source only).
    rig_dir: PathBuf,
}

impl FileSystemModelRegistry {
    /// Create a new filesystem-backed model registry.
    ///
    /// # Arguments
    ///
    /// * `rig_dir` - Path to the rig directory (e.g. `rig/`). The
    ///   registry file is read from `{rig_dir}/models.yml`.
    pub fn new(rig_dir: PathBuf) -> Self {
        Self { rig_dir }
    }

    /// Path of the registry file.
    fn models_path(&self) -> PathBuf {
        self.rig_dir.join("models.yml")
    }
}

impl ModelRegistryPort for FileSystemModelRegistry {
    fn load(&self) -> Result<ModelRegistry, PortError> {
        let path = self.models_path();

        // Missing file → empty registry (direct-spec profiles unaffected).
        if !path.exists() {
            return Ok(ModelRegistry::default());
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "WARNING: could not read model registry {}: {} — treating as empty registry",
                    path.display(),
                    e
                );
                return Ok(ModelRegistry::default());
            }
        };

        match ModelRegistry::from_yaml(&content) {
            Ok(registry) => Ok(registry),
            Err(e) => {
                eprintln!(
                    "WARNING: malformed model registry {}: {} — treating as empty registry",
                    path.display(),
                    e
                );
                Ok(ModelRegistry::default())
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write a models.yml file with one alias.
    fn write_models(rig_dir: &std::path::Path, content: &str) {
        fs::write(rig_dir.join("models.yml"), content).unwrap();
    }

    #[test]
    fn load_valid_registry() {
        let tmp = tempfile::tempdir().unwrap();
        write_models(
            tmp.path(),
            "models:\n  fast:\n    provider: openai\n    model: gpt-4o\n  frontier:\n    provider: anthropic\n    model: claude-sonnet-4-20250514\n",
        );

        let registry = FileSystemModelRegistry::new(tmp.path().to_path_buf())
            .load()
            .unwrap();

        assert_eq!(registry.len(), 2);
        let fast = registry.resolve("fast").unwrap();
        assert_eq!(fast.provider, "openai");
        assert_eq!(fast.model, "gpt-4o");
    }

    #[test]
    fn load_missing_file_returns_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        // No models.yml in the rig dir.

        let registry = FileSystemModelRegistry::new(tmp.path().to_path_buf())
            .load()
            .unwrap();

        assert!(registry.is_empty());
    }

    #[test]
    fn load_empty_file_returns_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        write_models(tmp.path(), "");

        let registry = FileSystemModelRegistry::new(tmp.path().to_path_buf())
            .load()
            .unwrap();

        assert!(registry.is_empty());
    }

    #[test]
    fn load_commented_template_returns_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        write_models(
            tmp.path(),
            "# Rig-level model registry.\n#\n# models:\n#   default:\n#     provider: openai\n#     model: gpt-4o\n",
        );

        let registry = FileSystemModelRegistry::new(tmp.path().to_path_buf())
            .load()
            .unwrap();

        assert!(registry.is_empty());
    }

    #[test]
    fn load_malformed_yaml_returns_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        write_models(tmp.path(), "models: [unclosed");

        let registry = FileSystemModelRegistry::new(tmp.path().to_path_buf())
            .load()
            .unwrap();

        assert!(registry.is_empty());
    }

    #[test]
    fn load_rejects_alias_with_empty_provider_as_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        write_models(
            tmp.path(),
            "models:\n  fast:\n    provider: \"\"\n    model: gpt-4o\n",
        );

        let registry = FileSystemModelRegistry::new(tmp.path().to_path_buf())
            .load()
            .unwrap();

        assert!(registry.is_empty());
    }

    #[test]
    fn load_is_fresh_per_call() {
        // The core lifetime guarantee: no caching. A rewrite of
        // models.yml between two load() calls is visible immediately.
        let tmp = tempfile::tempdir().unwrap();
        write_models(
            tmp.path(),
            "models:\n  fast:\n    provider: openai\n    model: gpt-4o\n",
        );

        let port = FileSystemModelRegistry::new(tmp.path().to_path_buf());
        let first = port.load().unwrap();
        assert_eq!(first.resolve("fast").unwrap().model, "gpt-4o");

        // Swap the model behind the alias.
        write_models(
            tmp.path(),
            "models:\n  fast:\n    provider: anthropic\n    model: claude-sonnet\n",
        );

        let second = port.load().unwrap();
        assert_eq!(
            second.resolve("fast").unwrap().model,
            "claude-sonnet"
        );
    }
}
