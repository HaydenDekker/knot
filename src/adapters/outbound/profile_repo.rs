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

}

impl AgentProfileRepository for FileSystemAgentProfileRepository {
    fn get(&self, name: &str) -> Result<Option<AgentProfile>, PortError> {
        let path = self.profiles_dir.join(format!("{name}.md"));

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
        Ok(Some(profile))
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
        profile_prompt: &str,
        tools: &[&str],
    ) {
        let content = if tools.is_empty() {
            format!(
                "---\nname: {name}\nprovider: {provider}\nmodel: {model}\n---\n\n{profile_prompt}\n"
            )
        } else {
            let tools_yaml: String = tools
                .iter()
                .map(|t| format!("  - {t}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "---\nname: {name}\nprovider: {provider}\nmodel: {model}\ntools:\n{tools_yaml}\n---\n\n{profile_prompt}\n"
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
        assert!(profile.profile_prompt.contains("fast reviewer"));
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
            "---\nprovider: openai\nmodel: gpt-4o\n---\n\nReview.\n",
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
