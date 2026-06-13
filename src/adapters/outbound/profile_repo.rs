//! Filesystem-backed implementation of `AgentProfileRepository`.
//!
//! Profiles are stored as `.md` files in `{rig}/profiles/{name}.md`
//! with YAML frontmatter. Files are parsed using `parse_agent_profile()`
//! from the domain layer at read time, so edits to profile files are
//! picked up on the next call without restart.

use std::fs;
use std::path::PathBuf;

use crate::application::ports::{
    AgentProfileRepository, PortError,
};
use crate::domain::knot_file::parse_agent_profile;
use crate::domain::value_objects::AgentProfile;

/// Extract the markdown body after the closing `---` delimiter in a
/// profile file. Returns `None` if the file has no body content.
fn extract_body(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = &trimmed[3..];
    let closing_pos = rest.find("---")?;
    let after = rest[closing_pos + 3..].trim();
    (!after.is_empty()).then(|| after.to_string())
}

/// Filesystem-backed implementation of `AgentProfileRepository`.
///
/// Profiles are stored in `{rig}/profiles/` with file naming
/// `{profile-name}.md`. The repository parses profile files at
/// read time, so edits are picked up dynamically.
#[derive(Clone)]
pub struct FileSystemAgentProfileRepository {
    /// Base directory where profiles are stored (typically `{rig}/profiles/`).
    profiles_dir: PathBuf,
}

impl FileSystemAgentProfileRepository {
    /// Create a new filesystem-backed profile repository.
    ///
    /// # Arguments
    ///
    /// * `profiles_dir` - Path to the profiles directory (e.g. `rig/profiles/`).
    pub fn new(profiles_dir: PathBuf) -> Self {
        Self { profiles_dir }
    }

    /// Ensure the profiles directory exists.
    ///
    /// Creates the directory and all parent directories if they don't
    /// exist. No-op if the directory already exists.
    fn ensure_dir(&self) -> Result<(), PortError> {
        fs::create_dir_all(&self.profiles_dir).map_err(|e| {
            PortError::ProfileSaveFailed(format!(
                "failed to create profiles directory {}: {}",
                self.profiles_dir.display(),
                e
            ))
        })
    }

    /// Build the file path for a profile by name.
    fn profile_path(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{name}.md"))
    }
}

impl AgentProfileRepository for FileSystemAgentProfileRepository {
    fn get(&self, name: &str) -> Result<Option<AgentProfile>, PortError> {
        let path = self.profile_path(name);

        // If the file doesn't exist, return None (not an error).
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path).map_err(|e| {
            PortError::ProfileNotFound(format!(
                "failed to read profile {}: {}",
                path.display(),
                e
            ))
        })?;

        let profile = parse_agent_profile(&content).map_err(|e| {
            PortError::ProfileNotFound(format!(
                "failed to parse profile {}: {}",
                path.display(),
                e
            ))
        })?;
        let body = extract_body(&content);
        Ok(Some(profile.with_body(body)))
    }

    fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
        // If the profiles directory doesn't exist, return empty list.
        if !self.profiles_dir.exists() {
            return Ok(Vec::new());
        }

        let entries = fs::read_dir(&self.profiles_dir).map_err(|e| {
            PortError::ProfileScanFailed(format!(
                "failed to read profiles directory {}: {}",
                self.profiles_dir.display(),
                e
            ))
        })?;

        let mut profiles = Vec::new();

        for entry_result in entries {
            let entry = entry_result.map_err(|e| {
                PortError::ProfileScanFailed(e.to_string())
            })?;

            let path = entry.path();

            // Only process .md files.
            match path.extension().and_then(|e| e.to_str()) {
                Some("md") => {}
                _ => continue,
            }

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "WARNING: skipping profile file {}: {}",
                        path.display(),
                        e
                    );
                    continue;
                }
            };

            match parse_agent_profile(&content) {
                Ok(profile) => profiles.push(profile),
                Err(e) => {
                    eprintln!(
                        "WARNING: skipping invalid profile {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        Ok(profiles)
    }

    fn save(&self, profile: AgentProfile) -> Result<(), PortError> {
        self.ensure_dir()?;

        let path = self.profile_path(&profile.name);

        // Extract existing markdown body if the file already exists.
        let preserved_body = if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => extract_body(&content),
                Err(e) => {
                    return Err(PortError::ProfileSaveFailed(format!(
                        "failed to read existing profile {}: {}",
                        path.display(),
                        e
                    )));
                }
            }
        } else {
            None
        };

        // Serialize the profile to YAML frontmatter.
        let yaml = serde_yaml::to_string(&profile).map_err(|e| {
            PortError::ProfileSaveFailed(format!(
                "failed to serialize profile {}: {}",
                profile.name,
                e
            ))
        })?;

        // Build content: frontmatter + preserved body (or default heading).
        let content = if let Some(body) = preserved_body {
            format!("---\n{yaml}---\n\n{body}\n")
        } else {
            format!(
                "---\n{yaml}---\n\n# {}\n\n{}\n",
                profile.name, profile.system_prompt
            )
        };

        fs::write(&path, content).map_err(|e| {
            PortError::ProfileSaveFailed(format!(
                "failed to write profile {}: {}",
                path.display(),
                e
            ))
        })
    }

    fn delete(&self, name: &str) -> Result<(), PortError> {
        let path = self.profile_path(name);

        if !path.exists() {
            return Err(PortError::ProfileNotFound(name.to_string()));
        }

        fs::remove_file(&path).map_err(|e| {
            PortError::ProfileNotFound(format!(
                "failed to delete profile {}: {}",
                name,
                e
            ))
        })?;

        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Create a valid profile file in the given directory.
    fn create_profile_file(
        dir: &Path,
        name: &str,
        provider: &str,
        model: &str,
        system_prompt: &str,
        tools: &[&str],
    ) {
        let content = if tools.is_empty() {
            format!(
                "---\nname: {name}\nprovider: {provider}\nmodel: {model}\nsystem-prompt: |\n  {system_prompt}\n---\n\n# {name}\n\nProfile {name}.\n"
            )
        } else {
            let tools_yaml: String = tools
                .iter()
                .map(|t| format!("  - {t}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "---\nname: {name}\nprovider: {provider}\nmodel: {model}\ntools:\n{tools_yaml}\nsystem-prompt: |\n  {system_prompt}\n---\n\n# {name}\n\nProfile {name}.\n"
            )
        };
        fs::write(dir.join(format!("{name}.md")), content).unwrap();
    }

    #[test]
    fn get_nonexistent_profile_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = FileSystemAgentProfileRepository::new(tmp.path().to_path_buf());

        let result = repo.get("does-not-exist");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn get_existing_profile_returns_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        create_profile_file(
            &profiles_dir,
            "fast",
            "openai",
            "gpt-4o",
            "You are a fast reviewer.",
            &["fs"],
        );

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.get("fast");
        assert!(result.is_ok());
        let profile = result.unwrap().unwrap();
        assert_eq!(profile.name, "fast");
        assert_eq!(profile.provider, "openai");
        assert_eq!(profile.model, "gpt-4o");
        assert_eq!(profile.tools, vec!["fs"]);
        assert!(profile.system_prompt.contains("fast reviewer"));
    }

    #[test]
    fn get_profile_from_nonexistent_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("nonexistent");
        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.get("some-profile");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn list_empty_profiles_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.list();
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn list_profiles_from_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("nonexistent");
        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.list();
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn list_multiple_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        create_profile_file(
            &profiles_dir,
            "fast",
            "openai",
            "gpt-4o",
            "You are fast.",
            &[],
        );
        create_profile_file(
            &profiles_dir,
            "detailed",
            "anthropic",
            "claude-sonnet",
            "You are detailed.",
            &["fs", "web"],
        );
        create_profile_file(
            &profiles_dir,
            "minimal",
            "openai",
            "gpt-3.5-turbo",
            "Be concise.",
            &[],
        );

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.list();
        assert!(result.is_ok());
        let profiles = result.unwrap();
        assert_eq!(profiles.len(), 3);

        let names: Vec<&str> =
            profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"fast"));
        assert!(names.contains(&"detailed"));
        assert!(names.contains(&"minimal"));
    }

    #[test]
    fn list_skips_non_md_files() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        // Create a valid profile.
        create_profile_file(
            &profiles_dir,
            "valid",
            "openai",
            "gpt-4o",
            "Valid profile.",
            &[],
        );

        // Create a non-.md file (should be skipped).
        fs::write(profiles_dir.join("ignored.txt"), "not a profile").unwrap();

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.list();
        assert!(result.is_ok());
        let profiles = result.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "valid");
    }

    #[test]
    fn list_skips_malformed_profile_files() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        // Create a valid profile.
        create_profile_file(
            &profiles_dir,
            "good",
            "openai",
            "gpt-4o",
            "Good profile.",
            &[],
        );

        // Create a malformed profile (no name).
        fs::write(
            profiles_dir.join("bad.md"),
            "---\nprovider: openai\nmodel: gpt-4o\nsystem-prompt: Review.\n---\n\nBad.\n",
        )
        .unwrap();

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.list();
        assert!(result.is_ok());
        let profiles = result.unwrap();
        assert_eq!(profiles.len(), 1, "malformed profile should be skipped");
        assert_eq!(profiles[0].name, "good");
    }

    #[test]
    fn save_creates_profiles_dir_and_file() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        // Directory does NOT exist yet.

        let repo = FileSystemAgentProfileRepository::new(profiles_dir.clone());

        let profile = AgentProfile::new(
            "new-profile".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are new.".to_string(),
        )
        .unwrap();

        let result = repo.save(profile);
        assert!(result.is_ok());

        // Verify the directory was created.
        assert!(profiles_dir.exists());

        // Verify the file was created.
        let path = profiles_dir.join("new-profile.md");
        assert!(path.exists());

        // Verify we can read it back.
        let loaded = repo.get("new-profile");
        assert!(loaded.is_ok());
        assert!(loaded.unwrap().is_some());
    }

    #[test]
    fn save_overwrites_existing_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        // Create an initial profile.
        let repo = FileSystemAgentProfileRepository::new(profiles_dir.clone());

        let profile1 = AgentProfile::new(
            "shared".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Original prompt.".to_string(),
        )
        .unwrap();
        repo.save(profile1).unwrap();

        // Verify initial content.
        let loaded = repo.get("shared").unwrap().unwrap();
        assert_eq!(loaded.model, "gpt-4o");

        // Overwrite with a different profile.
        let profile2 = AgentProfile::new(
            "shared".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Updated prompt.".to_string(),
        )
        .unwrap();
        repo.save(profile2).unwrap();

        // Verify the file was overwritten.
        let loaded = repo.get("shared").unwrap().unwrap();
        assert_eq!(loaded.provider, "anthropic");
        assert_eq!(loaded.model, "claude-sonnet");
        assert!(loaded.system_prompt.contains("Updated"));
    }

    #[test]
    fn save_overwrite_preserves_body() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        // Create initial profile with custom body.
        let repo = FileSystemAgentProfileRepository::new(profiles_dir.clone());

        let initial_content = "---\nname: shared\nprovider: openai\nmodel: gpt-4o\nsystem-prompt: |\n  Original prompt.\n---\n\n# Shared Profile\n\nThis is custom documentation that should be preserved across saves.\n";
        fs::write(profiles_dir.join("shared.md"), initial_content).unwrap();

        // Overwrite with new profile (different provider/model).
        let profile2 = AgentProfile::new(
            "shared".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "New prompt.".to_string(),
        )
        .unwrap();
        repo.save(profile2).unwrap();

        // Read raw file content and verify body is preserved.
        let file_content = fs::read_to_string(profiles_dir.join("shared.md")).unwrap();
        assert!(
            file_content.contains("This is custom documentation that should be preserved"),
            "body should be preserved after overwrite"
        );
        // Verify the data changed.
        let loaded = repo.get("shared").unwrap().unwrap();
        assert_eq!(loaded.provider, "anthropic");
        assert_eq!(loaded.model, "claude-sonnet");
    }

    #[test]
    fn save_with_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let profile = AgentProfile::with_tools(
            "full-stack".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            vec!["fs".to_string(), "web".to_string()],
            "Full stack review.".to_string(),
        )
        .unwrap();

        let result = repo.save(profile);
        assert!(result.is_ok());

        let loaded = repo.get("full-stack").unwrap().unwrap();
        assert_eq!(loaded.tools, vec!["fs", "web"]);
        assert_eq!(loaded.model, "gpt-4o");
    }

    #[test]
    fn delete_existing_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        create_profile_file(
            &profiles_dir,
            "to-delete",
            "openai",
            "gpt-4o",
            "Will be deleted.",
            &[],
        );

        let profile_path = profiles_dir.join("to-delete.md");
        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        // Verify profile exists.
        assert!(repo.get("to-delete").unwrap().is_some());

        // Delete it.
        let result = repo.delete("to-delete");
        assert!(result.is_ok());

        // Verify it's gone.
        assert!(repo.get("to-delete").unwrap().is_none());

        // Verify the file is deleted.
        assert!(!profile_path.exists());
    }

    #[test]
    fn delete_nonexistent_profile_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let result = repo.delete("does-not-exist");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PortError::ProfileNotFound("does-not-exist".to_string())
        );
    }

    #[test]
    fn delete_twice_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        create_profile_file(
            &profiles_dir,
            "gone",
            "openai",
            "gpt-4o",
            "Gone.",
            &[],
        );

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        // Delete once.
        assert!(repo.delete("gone").is_ok());

        // Delete again — should fail (not exist).
        assert!(repo.delete("gone").is_err());
    }

    #[test]
    fn save_then_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        let profile = AgentProfile::with_tools(
            "roundtrip".to_string(),
            "anthropic".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            vec!["fs".to_string()],
            "You are a thorough reviewer.\n\nKeep responses detailed.".to_string(),
        )
        .unwrap();

        repo.save(profile.clone()).unwrap();

        let loaded = repo.get("roundtrip").unwrap().unwrap();
        assert_eq!(loaded.name, profile.name);
        assert_eq!(loaded.provider, profile.provider);
        assert_eq!(loaded.model, profile.model);
        assert_eq!(loaded.tools, profile.tools);
        assert_eq!(loaded.system_prompt, profile.system_prompt);
    }

    #[test]
    fn full_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);

        // 1. List on empty repository.
        assert!(repo.list().unwrap().is_empty());

        // 2. Save a profile.
        let profile = AgentProfile::new(
            "lifecycle".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Lifecycle test.".to_string(),
        )
        .unwrap();
        assert!(repo.save(profile).is_ok());

        // 3. List should return one profile.
        let list = repo.list().unwrap();
        assert_eq!(list.len(), 1);

        // 4. Get should find it.
        assert!(repo.get("lifecycle").unwrap().is_some());

        // 5. Delete it.
        assert!(repo.delete("lifecycle").is_ok());

        // 6. List should be empty again.
        assert!(repo.list().unwrap().is_empty());

        // 7. Get should return None.
        assert!(repo.get("lifecycle").unwrap().is_none());
    }

    #[test]
    fn clone_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        let repo = FileSystemAgentProfileRepository::new(profiles_dir.clone());

        // FileSystemAgentProfileRepository should be Clone.
        let _repo2 = repo.clone();

        // Both should work.
        assert!(repo.list().is_ok());
    }
}
