use std::fs;
use std::path::PathBuf;

use crate::domain::events::PromptInstructionSource;
use crate::domain::value_objects::RigAgentConfig;

/// Filename of the rig-level agent config (shared with
/// `server::load_rig_config`).
const AGENT_CONFIG_FILE: &str = ".workspace-agent-config.yaml";

/// Filesystem-backed implementation of
/// [`PromptInstructionSource`](crate::domain::events::PromptInstructionSource)
/// (the tie-off / event-content instructions — the `knot-template`
/// skill's templates).
///
/// Everything is resolved fresh on every call — the instruction is
/// pulled on each new session and never cached from startup or a previous
/// run. Nothing is ever written into the rig:
///
/// 1. A link named in `rig/.workspace-agent-config.yaml`
///    (`tie-off-instruction-file` / `event-content-instruction-file`)
///    wins if present and readable — absolute path, relative to the rig
///    directory, `~/...` (expanded to `$HOME`), or a `file://` URL.
///    `http(s)://` values are refused with a warning (Knot never fetches
///    over the network).
/// 2. Otherwise `None` → the caller injects the hard-coded embedded
///    default (the template text compiled in via `include_str!` — the
///    `knot-template` skill's template files are its single source of
///    truth).
#[derive(Debug)]
pub struct FileSystemPromptInstructionSource {
    rig_dir: PathBuf,
    /// `$HOME` for `~/` link expansion (resolved at construction;
    /// injectable in tests).
    home_dir: Option<PathBuf>,
}

impl FileSystemPromptInstructionSource {
    /// Create a source rooted at `rig_dir` (the rig directory — contains
    /// `.workspace-agent-config.yaml`).
    pub fn new(rig_dir: PathBuf) -> Self {
        Self {
            rig_dir,
            home_dir: std::env::var("HOME").ok().map(PathBuf::from),
        }
    }

    /// Test constructor with an explicit `$HOME` (or `None` to simulate
    /// an environment without one).
    #[cfg(test)]
    pub fn with_home(rig_dir: PathBuf, home: Option<PathBuf>) -> Self {
        Self {
            rig_dir,
            home_dir: home,
        }
    }

    /// Shared resolution: linked file → `None` (the caller falls back to
    /// the hard-coded embedded default).
    fn read_instruction(&self, config_key: &str) -> Option<String> {
        let override_value = self.config_value(config_key)?;
        self.read_linked_file(config_key, &override_value)
    }

    /// Read the rig config (fresh, per call) and return the raw value of
    /// the given link key.
    fn config_value(&self, key: &str) -> Option<String> {
        let path = self.rig_dir.join(AGENT_CONFIG_FILE);
        if !path.is_file() {
            return None;
        }
        let content = fs::read_to_string(&path).ok()?;
        let config: RigAgentConfig = match serde_yaml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail=failed to parse {} ({e}) — using the hard-coded default",
                    AGENT_CONFIG_FILE
                );
                return None;
            }
        };
        match key {
            "tie-off-instruction-file" => config.tie_off_instruction_file,
            "event-content-instruction-file" => config.event_content_instruction_file,
            _ => None,
        }
    }

    /// Resolve and read a linked file. Absolute paths are used as-is;
    /// relative paths resolve against the rig directory; `~/...` expands
    /// to `$HOME`; `file://` URLs are converted to paths; `http(s)://`
    /// (and anything else non-file) is refused with a warning.
    fn read_linked_file(&self, key: &str, value: &str) -> Option<String> {
        let path = if value.starts_with("http://") || value.starts_with("https://") {
            eprintln!(
                "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} '{}' uses http(s) — only local paths and file:// URLs are supported; ignoring the link",
                key, value
            );
            return None;
        } else if let Some(rest) = value.strip_prefix("file://") {
            PathBuf::from(rest)
        } else if let Some(rest) = value.strip_prefix("~/") {
            match &self.home_dir {
                Some(home) => home.join(rest),
                None => {
                    eprintln!(
                        "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} '{}' uses ~/ but $HOME is not set; ignoring the link",
                        key, value
                    );
                    return None;
                }
            }
        } else {
            let p = PathBuf::from(value);
            if p.is_relative() {
                self.rig_dir.join(p)
            } else {
                p
            }
        };
        match fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(e) => {
                eprintln!(
                    "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} ('{}') missing or unreadable at {} ({e}) — using the hard-coded default",
                    key, value, path.display()
                );
                None
            }
        }
    }
}

impl PromptInstructionSource for FileSystemPromptInstructionSource {
    fn read_tie_off_instruction(&self) -> Option<String> {
        self.read_instruction("tie-off-instruction-file")
    }

    fn read_event_content_instruction(&self) -> Option<String> {
        self.read_instruction("event-content-instruction-file")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn temp_rig_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("knot-prompt-src-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn source(dir: &Path) -> FileSystemPromptInstructionSource {
        // HOME is never consulted unless a ~/ link is present, so the
        // real environment is harmless here.
        FileSystemPromptInstructionSource::new(dir.to_path_buf())
    }

    #[test]
    fn no_config_returns_none_embedded_default_applies() {
        let dir = temp_rig_dir("no-config");
        assert!(source(&dir).read_tie_off_instruction().is_none());
        assert!(source(&dir).read_event_content_instruction().is_none());
    }

    #[test]
    fn config_without_the_keys_returns_none() {
        let dir = temp_rig_dir("config-without-keys");
        write_file(&dir.join(AGENT_CONFIG_FILE), "agent-adapter: pi-json\n");
        assert!(source(&dir).read_tie_off_instruction().is_none());
        assert!(source(&dir).read_event_content_instruction().is_none());
    }

    #[test]
    fn relative_link_is_resolved_against_rig_dir() {
        let dir = temp_rig_dir("relative-link");
        write_file(&dir.join("knot-template").join("knot-tie-off.md"), "project override");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: knot-template/knot-tie-off.md",
        );
        assert_eq!(
            source(&dir).read_tie_off_instruction().as_deref(),
            Some("project override")
        );
    }

    #[test]
    fn absolute_link_is_supported() {
        let dir = temp_rig_dir("absolute");
        let custom = dir.join("custom-event-content.md");
        write_file(&custom, "via absolute path");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            &format!(
                "event-content-instruction-file: {}",
                custom.display()
            ),
        );
        assert_eq!(
            source(&dir)
                .read_event_content_instruction()
                .as_deref(),
            Some("via absolute path")
        );
    }

    #[test]
    fn file_url_link_is_supported() {
        let dir = temp_rig_dir("file-url");
        let custom = dir.join("custom-tieoff.md");
        write_file(&custom, "via file://");
        let url = format!("file://{}", custom.display());
        write_file(&dir.join(AGENT_CONFIG_FILE), &format!("tie-off-instruction-file: {url}"));
        assert_eq!(
            source(&dir).read_tie_off_instruction().as_deref(),
            Some("via file://")
        );
    }

    #[test]
    fn tilde_link_expands_to_home() {
        let rig = temp_rig_dir("tilde-rig");
        let home = temp_rig_dir("tilde-home");
        write_file(
            &home.join(".agents").join("skills-library").join("knot-tie-off.md"),
            "via ~/ link",
        );
        write_file(
            &rig.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: ~/.agents/skills-library/knot-tie-off.md",
        );
        let src = FileSystemPromptInstructionSource::with_home(rig, Some(home));
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("via ~/ link")
        );
    }

    #[test]
    fn tilde_link_without_home_is_refused() {
        let rig = temp_rig_dir("tilde-no-home");
        write_file(
            &rig.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: ~/anywhere.md",
        );
        let src = FileSystemPromptInstructionSource::with_home(rig, None);
        assert!(src.read_tie_off_instruction().is_none());
    }

    #[test]
    fn http_link_is_refused_returns_none() {
        let dir = temp_rig_dir("http-refused");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: https://example.com/tie-off.md",
        );
        assert!(source(&dir).read_tie_off_instruction().is_none());
    }

    #[test]
    fn missing_link_returns_none() {
        let dir = temp_rig_dir("link-missing");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "event-content-instruction-file: does-not-exist.md",
        );
        assert!(source(&dir).read_event_content_instruction().is_none());
    }

    #[test]
    fn link_does_not_leak_across_keys() {
        let dir = temp_rig_dir("link-scoped");
        write_file(&dir.join("custom.md"), "tie-off link only");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: custom.md",
        );
        assert_eq!(
            source(&dir).read_tie_off_instruction().as_deref(),
            Some("tie-off link only")
        );
        // The event-content key is unset → no link → None (the domain
        // then uses the hard-coded default).
        assert!(source(&dir).read_event_content_instruction().is_none());
    }

    #[test]
    fn content_is_reread_on_every_call() {
        let dir = temp_rig_dir("reread");
        let linked = dir.join("knot-template").join("knot-tie-off.md");
        write_file(&linked, "version 1");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: knot-template/knot-tie-off.md",
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("version 1")
        );
        // The user edits the linked file — the next session (next call)
        // must see the new content (never cached).
        fs::write(&linked, "version 2").unwrap();
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("version 2")
        );
    }

    #[test]
    fn malformed_config_returns_none() {
        let dir = temp_rig_dir("malformed-config");
        write_file(&dir.join(AGENT_CONFIG_FILE), "not: valid: yaml: [");
        assert!(source(&dir).read_tie_off_instruction().is_none());
    }

    #[test]
    fn config_reread_on_every_call() {
        let dir = temp_rig_dir("config-reread");
        write_file(&dir.join("custom.md"), "newly linked");
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        // The user adds the link after the first session — the next
        // session (next call) must pick it up (no restart needed).
        fs::write(dir.join(AGENT_CONFIG_FILE), "tie-off-instruction-file: custom.md").unwrap();
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("newly linked")
        );
    }
}
