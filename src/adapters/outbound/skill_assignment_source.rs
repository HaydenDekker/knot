//! Filesystem-backed [`SkillAssignmentSource`] (plan 105).
//!
//! Reads the rig-level `.workspace-agent-config.yaml` **fresh on every
//! call** — the `skills` list and `skills-mode` are never cached from
//! startup or a previous run (the plan-102
//! [`FileSystemPromptInstructionSource`] precedent). A project can
//! therefore add, re-point, or remove a skill import and the next knot
//! session picks it up with no service restart.

use std::fs;
use std::path::PathBuf;

use crate::domain::skill_assignment::{SkillAssignmentSource, SkillDeclarations};
use crate::domain::value_objects::RigAgentConfig;

/// Filename of the rig-level agent config (shared with
/// `server::load_rig_config` and the prompt instruction source).
const AGENT_CONFIG_FILE: &str = ".workspace-agent-config.yaml";

pub struct FileSystemSkillAssignmentSource {
    /// The **local** rig directory (contains
    /// `.workspace-agent-config.yaml`); also the resolution root for
    /// rig-config `skills` entries.
    rig_dir: PathBuf,
}

impl FileSystemSkillAssignmentSource {
    /// Create a source rooted at the **local** rig directory.
    pub fn new(rig_dir: PathBuf) -> Self {
        Self { rig_dir }
    }

    /// Read the rig config fresh — `None` when the file is missing or
    /// unparseable (a malformed config warns; the session then keeps
    /// full discovery, unchanged behaviour).
    fn parse_config(&self) -> Option<RigAgentConfig> {
        let path = self.rig_dir.join(AGENT_CONFIG_FILE);
        if !path.is_file() {
            return None;
        }
        let content = fs::read_to_string(&path).ok()?;
        match serde_yaml::from_str(&content) {
            Ok(config) => Some(config),
            Err(e) => {
                eprintln!(
                    "[KNOT][EVENT] type=CONFIG source=skill-assignment detail=failed to parse {} ({e}) — keeping full skill discovery for this session",
                    AGENT_CONFIG_FILE
                );
                None
            }
        }
    }
}

impl std::fmt::Debug for FileSystemSkillAssignmentSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSystemSkillAssignmentSource")
            .field("rig_dir", &self.rig_dir)
            .finish()
    }
}

impl SkillAssignmentSource for FileSystemSkillAssignmentSource {
    /// Fresh read per call: the `skills` list plus a lenient
    /// `skills-mode` interpretation — `exclusive` narrows, `additive`
    /// (or absent) extends, anything else warns and extends (an
    /// invalid mode never fails the session).
    fn read_skill_declarations(&self) -> SkillDeclarations {
        let Some(config) = self.parse_config() else {
            return SkillDeclarations::default();
        };
        let exclusive = match config.skills_mode.as_deref() {
            None | Some("additive") => false,
            Some("exclusive") => true,
            Some(other) => {
                eprintln!(
                    "[KNOT][EVENT] type=CONFIG source=skill-assignment detail=skills-mode '{}' is not 'additive' or 'exclusive' — falling back to additive for this session",
                    other
                );
                false
            }
        };
        SkillDeclarations {
            rig_entries: config.skills,
            exclusive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn temp_rig(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("knot-skill-src-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, content: &str) {
        fs::write(dir.join(AGENT_CONFIG_FILE), content).unwrap();
    }

    #[test]
    fn no_config_means_additive_empty() {
        let dir = temp_rig("no-config");
        let src = FileSystemSkillAssignmentSource::new(dir);
        let decl = src.read_skill_declarations();
        assert_eq!(decl, SkillDeclarations::default());
        assert!(!decl.exclusive);
    }

    #[test]
    fn config_without_skill_keys_is_unchanged() {
        let dir = temp_rig("no-keys");
        write(&dir, "agent-adapter: pi-json\n");
        let src = FileSystemSkillAssignmentSource::new(dir);
        let decl = src.read_skill_declarations();
        assert!(decl.rig_entries.is_empty());
        assert!(!decl.exclusive);
    }

    #[test]
    fn skills_list_and_exclusive_mode_parse() {
        let dir = temp_rig("full");
        write(
            &dir,
            "skills:\n  - theme-contract/skill.md\n  - ~/shared/review.md\nskills-mode: exclusive\n",
        );
        let src = FileSystemSkillAssignmentSource::new(dir);
        let decl = src.read_skill_declarations();
        assert_eq!(decl.rig_entries.len(), 2);
        assert_eq!(decl.rig_entries[0], "theme-contract/skill.md");
        assert!(decl.exclusive);
    }

    #[test]
    fn additive_mode_is_explicit_no_op() {
        let dir = temp_rig("additive");
        write(&dir, "skills: [a.md]\nskills-mode: additive\n");
        let src = FileSystemSkillAssignmentSource::new(dir);
        let decl = src.read_skill_declarations();
        assert_eq!(decl.rig_entries, vec!["a.md".to_string()]);
        assert!(!decl.exclusive);
    }

    #[test]
    fn invalid_mode_falls_back_to_additive() {
        let dir = temp_rig("bad-mode");
        write(&dir, "skills: [a.md]\nskills-mode: sideways\n");
        let src = FileSystemSkillAssignmentSource::new(dir);
        let decl = src.read_skill_declarations();
        assert_eq!(decl.rig_entries, vec!["a.md".to_string()]);
        assert!(!decl.exclusive, "an invalid mode must not narrow the session");
    }

    #[test]
    fn malformed_config_keeps_full_discovery() {
        let dir = temp_rig("malformed");
        write(&dir, "skills: [unclosed\n");
        let src = FileSystemSkillAssignmentSource::new(dir);
        assert_eq!(src.read_skill_declarations(), SkillDeclarations::default());
    }

    #[test]
    fn re_read_per_call_picks_up_edits() {
        let dir = temp_rig("fresh");
        write(&dir, "skills: [a.md]\n");
        let src = FileSystemSkillAssignmentSource::new(dir.clone());
        assert_eq!(src.read_skill_declarations().rig_entries, vec!["a.md"]);
        // A project edit lands on the next read — no restart.
        write(&dir, "skills: [b.md]\nskills-mode: exclusive\n");
        let decl = src.read_skill_declarations();
        assert_eq!(decl.rig_entries, vec!["b.md"]);
        assert!(decl.exclusive);
    }
}
