use std::fs;
use std::path::PathBuf;

use crate::domain::events::PromptInstructionSource;
use crate::domain::value_objects::RigAgentConfig;

/// Filename of the rig-level agent config (shared with
/// `server::load_rig_config`).
const AGENT_CONFIG_FILE: &str = ".workspace-agent-config.yaml";

/// The config key for the tie-off content section — the one
/// overridable instruction (the `knot-template` skill's content
/// template).
const CONTENT_KEY: &str = "tie-off-content-instruction-file";

/// The 0.59.0 keys, retired in 0.60.0: kept parse-only in
/// [`RigAgentConfig`] so a rig's leftover keys can be detected and
/// warned about. The tie-off contract is hard-coded (never honoured)
/// and the event-content guide moved into the content section.
const DEPRECATED_TIE_OFF_KEY: &str = "tie-off-instruction-file";
const DEPRECATED_EVENT_CONTENT_KEY: &str = "event-content-instruction-file";

/// Filesystem-backed implementation of
/// [`PromptInstructionSource`](crate::domain::events::PromptInstructionSource)
/// (the tie-off **content section** — the `knot-template` skill's
/// content template).
///
/// The hard-coded core (the tie-off contract, the event format, and the
/// rules — [`DEFAULT_TIE_OFF_INSTRUCTION`](crate::domain::events::DEFAULT_TIE_OFF_INSTRUCTION))
/// is never sourced here; only the content section is overridable per
/// rig.
///
/// Everything is resolved fresh on every call — the instruction is
/// pulled on each new session and never cached from startup or a
/// previous run. Nothing is ever written into the rig:
///
/// 1. A link named in `rig/.workspace-agent-config.yaml`
///    (`tie-off-content-instruction-file`) wins if present and
///    readable — absolute path, relative to the rig directory, `~/...`
///    (expanded to `$HOME`), or a `file://` URL. `http(s)://` values
///    are refused with a warning (Knot never fetches over the
///    network).
/// 2. Otherwise `None` → the caller injects the hard-coded embedded
///    default content section (the template text compiled in via
///    `include_str!` — the `knot-template` skill's content template is
///    its single source of truth).
///
/// The 0.59.0 keys (`tie-off-instruction-file` /
/// `event-content-instruction-file`) are retired: their presence is
/// reported with a `[KNOT][EVENT] type=CONFIG` warning (on every read —
/// the config is re-read per session) and they are never honoured.
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

    /// Shared resolution: linked file → `None` (the caller falls back
    /// to the hard-coded embedded default content section).
    fn read_instruction(&self) -> Option<String> {
        let override_value = self.content_link()?;
        self.read_linked_file(&override_value)
    }

    /// Read the rig config (fresh, per call) and return the content
    /// section's link value — or `None` when the key is absent.
    ///
    /// Warns (per read) about the two retired 0.59.0 keys when present —
    /// they are parse-only and never honoured.
    fn content_link(&self) -> Option<String> {
        let path = self.rig_dir.join(AGENT_CONFIG_FILE);
        if !path.is_file() {
            return None;
        }
        let content = fs::read_to_string(&path).ok()?;
        let config: RigAgentConfig = match serde_yaml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail=failed to parse {} ({e}) — using the hard-coded default content section",
                    AGENT_CONFIG_FILE
                );
                return None;
            }
        };
        if config.deprecated_tie_off_instruction_file.is_some() {
            eprintln!(
                "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} is deprecated in Knot 0.60.0 and no longer honoured — the tie-off contract, event format, and rules are hard-coded; remove the key",
                DEPRECATED_TIE_OFF_KEY
            );
        }
        if config.deprecated_event_content_instruction_file.is_some() {
            eprintln!(
                "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} is deprecated in Knot 0.60.0 — relink the file under {} (the tie-off content section)",
                DEPRECATED_EVENT_CONTENT_KEY, CONTENT_KEY
            );
        }
        config.tie_off_content_instruction_file
    }

    /// Resolve and read a linked file. Absolute paths are used as-is;
    /// relative paths resolve against the rig directory; `~/...` expands
    /// to `$HOME`; `file://` URLs are converted to paths; `http(s)://`
    /// (and anything else non-file) is refused with a warning.
    fn read_linked_file(&self, value: &str) -> Option<String> {
        let path = if value.starts_with("http://") || value.starts_with("https://") {
            eprintln!(
                "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} '{}' uses http(s) — only local paths and file:// URLs are supported; ignoring the link",
                CONTENT_KEY, value
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
                        CONTENT_KEY, value
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
                    "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} ('{}') missing or unreadable at {} ({e}) — using the hard-coded default content section",
                    CONTENT_KEY, value, path.display()
                );
                None
            }
        }
    }
}

impl PromptInstructionSource for FileSystemPromptInstructionSource {
    fn read_tie_off_content_instruction(&self) -> Option<String> {
        self.read_instruction()
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
        assert!(source(&dir).read_tie_off_content_instruction().is_none());
    }

    #[test]
    fn config_without_the_key_returns_none() {
        let dir = temp_rig_dir("config-without-key");
        write_file(&dir.join(AGENT_CONFIG_FILE), "agent-adapter: pi-json\n");
        assert!(source(&dir).read_tie_off_content_instruction().is_none());
    }

    #[test]
    fn relative_link_is_resolved_against_rig_dir() {
        let dir = temp_rig_dir("relative-link");
        write_file(
            &dir.join("knot-template").join("knot-tie-off-content.md"),
            "project override",
        );
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: knot-template/knot-tie-off-content.md",
        );
        assert_eq!(
            source(&dir).read_tie_off_content_instruction().as_deref(),
            Some("project override")
        );
    }

    #[test]
    fn absolute_link_is_supported() {
        let dir = temp_rig_dir("absolute");
        let custom = dir.join("custom-content.md");
        write_file(&custom, "via absolute path");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            &format!(
                "tie-off-content-instruction-file: {}",
                custom.display()
            ),
        );
        assert_eq!(
            source(&dir).read_tie_off_content_instruction().as_deref(),
            Some("via absolute path")
        );
    }

    #[test]
    fn file_url_link_is_supported() {
        let dir = temp_rig_dir("file-url");
        let custom = dir.join("custom-content.md");
        write_file(&custom, "via file://");
        let url = format!("file://{}", custom.display());
        write_file(&dir.join(AGENT_CONFIG_FILE), &format!("tie-off-content-instruction-file: {url}"));
        assert_eq!(
            source(&dir).read_tie_off_content_instruction().as_deref(),
            Some("via file://")
        );
    }

    #[test]
    fn tilde_link_expands_to_home() {
        let rig = temp_rig_dir("tilde-rig");
        let home = temp_rig_dir("tilde-home");
        write_file(
            &home
                .join(".agents")
                .join("skills-library")
                .join("knot-tie-off-content.md"),
            "via ~/ link",
        );
        write_file(
            &rig.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: ~/.agents/skills-library/knot-tie-off-content.md",
        );
        let src = FileSystemPromptInstructionSource::with_home(rig, Some(home));
        assert_eq!(
            src.read_tie_off_content_instruction().as_deref(),
            Some("via ~/ link")
        );
    }

    #[test]
    fn tilde_link_without_home_is_refused() {
        let rig = temp_rig_dir("tilde-no-home");
        write_file(
            &rig.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: ~/anywhere.md",
        );
        let src = FileSystemPromptInstructionSource::with_home(rig, None);
        assert!(src.read_tie_off_content_instruction().is_none());
    }

    #[test]
    fn http_link_is_refused_returns_none() {
        let dir = temp_rig_dir("http-refused");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: https://example.com/content.md",
        );
        assert!(source(&dir).read_tie_off_content_instruction().is_none());
    }

    #[test]
    fn missing_link_returns_none() {
        let dir = temp_rig_dir("link-missing");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: does-not-exist.md",
        );
        assert!(source(&dir).read_tie_off_content_instruction().is_none());
    }

    /// The retired 0.59.0 keys are parse-only: their linked files exist,
    /// but neither value is read as the content section.
    #[test]
    fn deprecated_keys_are_not_honoured() {
        let dir = temp_rig_dir("deprecated-keys");
        write_file(&dir.join("old-tieoff.md"), "tie-off link value");
        write_file(&dir.join("old-event-content.md"), "event-content link value");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: old-tieoff.md\n\
             event-content-instruction-file: old-event-content.md\n",
        );
        assert!(
            source(&dir).read_tie_off_content_instruction().is_none(),
            "the deprecated keys must not feed the content section"
        );
    }

    /// The deprecated keys coexist with the new key — the new key wins;
    /// the old values are never read.
    #[test]
    fn new_key_wins_over_deprecated_keys() {
        let dir = temp_rig_dir("new-key-wins");
        write_file(&dir.join("new-content.md"), "new key value");
        write_file(&dir.join("old-tieoff.md"), "tie-off link value");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: old-tieoff.md\n\
             tie-off-content-instruction-file: new-content.md\n",
        );
        assert_eq!(
            source(&dir).read_tie_off_content_instruction().as_deref(),
            Some("new key value")
        );
    }

    #[test]
    fn content_is_reread_on_every_call() {
        let dir = temp_rig_dir("reread");
        let linked = dir.join("knot-template").join("knot-tie-off-content.md");
        write_file(&linked, "version 1");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: knot-template/knot-tie-off-content.md",
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_content_instruction().as_deref(),
            Some("version 1")
        );
        // The user edits the linked file — the next session (next call)
        // must see the new content (never cached).
        fs::write(&linked, "version 2").unwrap();
        assert_eq!(
            src.read_tie_off_content_instruction().as_deref(),
            Some("version 2")
        );
    }

    #[test]
    fn malformed_config_returns_none() {
        let dir = temp_rig_dir("malformed-config");
        write_file(&dir.join(AGENT_CONFIG_FILE), "not: valid: yaml: [");
        assert!(source(&dir).read_tie_off_content_instruction().is_none());
    }

    #[test]
    fn config_reread_on_every_call() {
        let dir = temp_rig_dir("config-reread");
        write_file(&dir.join("custom.md"), "newly linked");
        let src = source(&dir);
        assert!(src.read_tie_off_content_instruction().is_none());
        // The user adds the link after the first session — the next
        // session (next call) must pick it up (no restart needed).
        fs::write(
            dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: custom.md",
        )
        .unwrap();
        assert_eq!(
            src.read_tie_off_content_instruction().as_deref(),
            Some("newly linked")
        );
    }
}
