use std::fs;
use std::path::PathBuf;

use crate::domain::events::PromptInstructionSource;
use crate::domain::value_objects::RigAgentConfig;

/// Filename of the rig-level agent config (shared with
/// `server::load_rig_config`).
const AGENT_CONFIG_FILE: &str = ".workspace-agent-config.yaml";

/// The config key for doc 1 — the tie-off instruction (the `##
/// Tie-Off Content` section only).
const TIE_OFF_KEY: &str = "tie-off-instruction-file";

/// The config key for doc 2 — the event-content instruction (the `##
/// Event Content Guide` section only).
const EVENT_CONTENT_KEY: &str = "event-content-instruction-file";

/// The 0.60.0–0.64.x combined content-section key, retired in 0.65.0:
/// kept parse-only in [`RigAgentConfig`] so a rig's leftover key can be
/// detected and warned about. Never honoured.
const RETIRED_COMBINED_KEY: &str = "tie-off-content-instruction-file";

/// Filesystem-backed implementation of
/// [`PromptInstructionSource`](crate::domain::events::PromptInstructionSource)
/// (the two overridable instruction **documents** — the `knot-template`
/// skill's two content templates).
///
/// The two documents, each with a fixed injection scope:
///
/// - **Doc 1 — the tie-off instruction** (`read_tie_off_instruction`,
///   the `## Tie-Off Content` section): injected into **every** knot,
///   every new session (unconditional — the `# Tie-Off` block).
/// - **Doc 2 — the event-content instruction**
///   (`read_event_content_instruction`, the `## Event Content Guide`
///   section): injected only when the knot has subscriber events (the
///   `# Subscriber Events` block).
///
/// The hard-coded cores — the universal tie-off contract sentence
/// (`DEFAULT_TIE_OFF_CONTRACT`) and the event acknowledgement / format
/// / rules (`DEFAULT_EVENT_CONTRACT`) — are never sourced here; only
/// the two content sections are overridable per rig.
///
/// Everything is resolved fresh on every call — each instruction is
/// pulled on each new session and never cached from startup or a
/// previous run. Nothing is ever written into the rig:
///
/// 1. A link named in `rig/.workspace-agent-config.yaml` (per key)
///    wins if present and readable — absolute path, relative to the
///    rig directory, `~/...` (expanded to `$HOME`), or a `file://`
///    URL. `http(s)://` values are refused with a warning (Knot never
///    fetches over the network).
/// 2. Otherwise `None` → the caller injects the hard-coded embedded
///    default for **that section** (the template text compiled in via
///    `include_str!` — the `knot-template` skill's templates are their
///    single source of truth). The other section is unaffected
///    (per-section fallback — independently per key).
///
/// The retired 0.60.0–0.64.x combined key
/// (`tie-off-content-instruction-file`) is parse-only: its presence is
/// reported with a `[KNOT][EVENT] type=CONFIG` warning (on every read —
/// the config is re-read per session) and its value is never read as
/// content.
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

    /// Read the rig config (fresh, per call) — `None` when the file is
    /// missing or unparseable (malformed config warns).
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
                    "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail=failed to parse {} ({e}) — using the hard-coded defaults for both sections",
                    AGENT_CONFIG_FILE
                );
                None
            }
        }
    }

    /// Warn (per read) about the retired combined key when present —
    /// it is parse-only and never honoured.
    fn warn_retired_key(&self, config: &RigAgentConfig) {
        if config.tie_off_content_instruction_file.is_some() {
            eprintln!(
                "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} is retired in Knot 0.65.0 and no longer honoured — split the linked file at the `## Event Content Guide` heading: the top half re-links under {}, the bottom half under {}",
                RETIRED_COMBINED_KEY, TIE_OFF_KEY, EVENT_CONTENT_KEY
            );
        }
    }

    /// Resolve and read a linked file for the named key. `None` value
    /// (key absent) → `None` (the caller falls back to the hard-coded
    /// embedded default for that section). Absolute paths are used
    /// as-is; relative paths resolve against the rig directory; `~/...`
    /// expands to `$HOME`; `file://` URLs are converted to paths;
    /// `http(s)://` (and anything else non-file) is refused with a
    /// warning.
    fn read_linked_file(&self, key: &str, value: Option<&str>) -> Option<String> {
        let value = value?;
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
                    "[KNOT][EVENT] type=CONFIG source=prompt-instructions detail={} ('{}') missing or unreadable at {} ({e}) — using the hard-coded default for that section",
                    key, value, path.display()
                );
                None
            }
        }
    }
}

impl PromptInstructionSource for FileSystemPromptInstructionSource {
    /// Doc 1 — the tie-off instruction (the `## Tie-Off Content`
    /// section). Fresh config parse per call; the retired combined key
    /// warns per call; the value is never read.
    fn read_tie_off_instruction(&self) -> Option<String> {
        let config = self.parse_config()?;
        self.warn_retired_key(&config);
        self.read_linked_file(TIE_OFF_KEY, config.tie_off_instruction_file.as_deref())
    }

    /// Doc 2 — the event-content instruction (the `## Event Content
    /// Guide` section). Independent of the tie-off instruction: a
    /// failure of one link never affects the other read.
    fn read_event_content_instruction(&self) -> Option<String> {
        let config = self.parse_config()?;
        self.warn_retired_key(&config);
        self.read_linked_file(
            EVENT_CONTENT_KEY,
            config.event_content_instruction_file.as_deref(),
        )
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

    // ── key absent / config absent → None (embedded default applies) ──

    #[test]
    fn no_config_returns_none_for_both_sections() {
        let dir = temp_rig_dir("no-config");
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }

    #[test]
    fn config_without_keys_returns_none_for_both_sections() {
        let dir = temp_rig_dir("config-without-keys");
        write_file(&dir.join(AGENT_CONFIG_FILE), "agent-adapter: pi-json\n");
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }

    // ── each honoured key: all four link forms ─────────────────────────

    #[test]
    fn tie_off_key_relative_link_is_resolved_against_rig_dir() {
        let dir = temp_rig_dir("tieoff-relative-link");
        write_file(
            &dir.join("knot-template").join("knot-tie-off-content.md"),
            "project tie-off override",
        );
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: knot-template/knot-tie-off-content.md",
        );
        assert_eq!(
            source(&dir).read_tie_off_instruction().as_deref(),
            Some("project tie-off override")
        );
    }

    #[test]
    fn event_content_key_relative_link_is_resolved_against_rig_dir() {
        let dir = temp_rig_dir("event-content-relative-link");
        write_file(
            &dir.join("knot-template").join("knot-event-content.md"),
            "project event-content override",
        );
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "event-content-instruction-file: knot-template/knot-event-content.md",
        );
        assert_eq!(
            source(&dir).read_event_content_instruction().as_deref(),
            Some("project event-content override")
        );
    }

    #[test]
    fn absolute_link_is_supported_for_both_keys() {
        let dir = temp_rig_dir("absolute");
        write_file(
            &dir.join("custom-tie-off.md"),
            "tie-off via absolute path",
        );
        write_file(
            &dir.join("custom-event-content.md"),
            "event-content via absolute path",
        );
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            &format!(
                "tie-off-instruction-file: {}\n\
                 event-content-instruction-file: {}\n",
                dir.join("custom-tie-off.md").display(),
                dir.join("custom-event-content.md").display()
            ),
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off via absolute path")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event-content via absolute path")
        );
    }

    #[test]
    fn file_url_link_is_supported_for_both_keys() {
        let dir = temp_rig_dir("file-url");
        write_file(&dir.join("tie-off.md"), "tie-off via file://");
        write_file(&dir.join("event-content.md"), "event-content via file://");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            &format!(
                "tie-off-instruction-file: file://{}\n\
                 event-content-instruction-file: file://{}\n",
                dir.join("tie-off.md").display(),
                dir.join("event-content.md").display()
            ),
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off via file://")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event-content via file://")
        );
    }

    #[test]
    fn tilde_link_expands_to_home_for_both_keys() {
        let rig = temp_rig_dir("tilde-rig");
        let home = temp_rig_dir("tilde-home");
        write_file(
            &home
                .join(".agents")
                .join("skills-library")
                .join("knot-tie-off-content.md"),
            "tie-off via ~/ link",
        );
        write_file(
            &home
                .join(".agents")
                .join("skills-library")
                .join("knot-event-content.md"),
            "event-content via ~/ link",
        );
        write_file(
            &rig.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: ~/.agents/skills-library/knot-tie-off-content.md\n\
             event-content-instruction-file: ~/.agents/skills-library/knot-event-content.md\n",
        );
        let src = FileSystemPromptInstructionSource::with_home(rig, Some(home));
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off via ~/ link")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event-content via ~/ link")
        );
    }

    #[test]
    fn tilde_link_without_home_is_refused_per_key() {
        let rig = temp_rig_dir("tilde-no-home");
        write_file(
            &rig.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: ~/tie-off.md\n\
             event-content-instruction-file: ~/event-content.md\n",
        );
        let src = FileSystemPromptInstructionSource::with_home(rig, None);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }

    #[test]
    fn http_link_is_refused_per_key() {
        let dir = temp_rig_dir("http-refused");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: https://example.com/tie-off.md\n\
             event-content-instruction-file: http://example.com/event-content.md\n",
        );
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }

    #[test]
    fn missing_link_returns_none_per_key() {
        let dir = temp_rig_dir("link-missing");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: does-not-exist.md\n\
             event-content-instruction-file: does-not-exist-either.md\n",
        );
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }

    // ── per-section fallback: no leak across keys ──────────────────────

    /// A failure of one link never affects the other read: the tie-off
    /// link is missing (→ `None`), the event-content link is healthy
    /// (→ `Some`).
    #[test]
    fn one_failed_link_does_not_affect_the_other_read() {
        let dir = temp_rig_dir("cross-key-fallback");
        write_file(&dir.join("event-content.md"), "event-content value");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: does-not-exist.md\n\
             event-content-instruction-file: knot-event-content.md\n",
        );
        // The event-content file lives at the rig root (relative to the
        // rig dir).
        write_file(&dir.join("knot-event-content.md"), "event-content value");
        let src = source(&dir);
        assert!(
            src.read_tie_off_instruction().is_none(),
            "missing tie-off link → None (embedded default for that section)"
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event-content value"),
            "the healthy event-content link must be honoured"
        );
    }

    /// Each section falls back to its own embedded default on its own
    /// link failure (observable here as `None` per key — the defaults
    /// themselves are the `include_str!`-ed templates).
    #[test]
    fn each_section_falls_back_independently() {
        let dir = temp_rig_dir("independent-fallback");
        write_file(&dir.join("tie-off.md"), "tie-off value");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: tie-off.md\n\
             event-content-instruction-file: missing.md\n",
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off value")
        );
        assert!(
            src.read_event_content_instruction().is_none(),
            "failed event-content link → None, independent of the tie-off read"
        );
    }

    // ── the retired 0.60.0–0.64.x combined key ─────────────────────────

    /// The retired combined key is parse-only: its linked file exists,
    /// but its value is never read as the content of either section.
    #[test]
    fn retired_combined_key_is_not_honoured() {
        let dir = temp_rig_dir("retired-key");
        write_file(&dir.join("combined.md"), "combined section value");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: combined.md\n",
        );
        let src = source(&dir);
        assert!(
            src.read_tie_off_instruction().is_none(),
            "the retired key must not feed the tie-off instruction"
        );
        assert!(
            src.read_event_content_instruction().is_none(),
            "the retired key must not feed the event-content instruction"
        );
    }

    /// A 0.60.0-era config (only the combined key) parses unchanged —
    /// all keys absent-default `None`, nothing honoured.
    #[test]
    fn pre_065_config_parses_unchanged() {
        let dir = temp_rig_dir("pre-065-config");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "agent-adapter: pi-json\n\
             tie-off-content-instruction-file: knot-template/knot-tie-off-content.md\n",
        );
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }

    /// The retired key coexists with the two new keys — the new keys
    /// win; the retired value is never read.
    #[test]
    fn new_keys_win_over_retired_key() {
        let dir = temp_rig_dir("new-keys-win");
        write_file(&dir.join("tie-off.md"), "tie-off new key value");
        write_file(&dir.join("event-content.md"), "event-content new key value");
        write_file(&dir.join("combined.md"), "retired key value");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-content-instruction-file: combined.md\n\
             tie-off-instruction-file: tie-off.md\n\
             event-content-instruction-file: event-content.md\n",
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off new key value")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event-content new key value")
        );
    }

    // ── fresh re-read per call (config and linked files) ───────────────

    #[test]
    fn content_is_reread_on_every_call_per_key() {
        let dir = temp_rig_dir("reread");
        let tie_off_link = dir.join("knot-template").join("knot-tie-off-content.md");
        let event_link = dir.join("knot-template").join("knot-event-content.md");
        write_file(&tie_off_link, "tie-off version 1");
        write_file(&event_link, "event version 1");
        write_file(
            &dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: knot-template/knot-tie-off-content.md\n\
             event-content-instruction-file: knot-template/knot-event-content.md\n",
        );
        let src = source(&dir);
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off version 1")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event version 1")
        );
        // The user edits the linked files — the next session (next call)
        // must see the new content (never cached).
        fs::write(&tie_off_link, "tie-off version 2").unwrap();
        fs::write(&event_link, "event version 2").unwrap();
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off version 2")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event version 2")
        );
    }

    #[test]
    fn config_reread_on_every_call_per_key() {
        let dir = temp_rig_dir("config-reread");
        write_file(&dir.join("tie-off.md"), "tie-off newly linked");
        write_file(&dir.join("event-content.md"), "event newly linked");
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
        // The user adds the links after the first session — the next
        // session (next call) must pick them up (no restart needed).
        fs::write(
            dir.join(AGENT_CONFIG_FILE),
            "tie-off-instruction-file: tie-off.md\n\
             event-content-instruction-file: event-content.md\n",
        )
        .unwrap();
        assert_eq!(
            src.read_tie_off_instruction().as_deref(),
            Some("tie-off newly linked")
        );
        assert_eq!(
            src.read_event_content_instruction().as_deref(),
            Some("event newly linked")
        );
    }

    #[test]
    fn malformed_config_returns_none_for_both_sections() {
        let dir = temp_rig_dir("malformed-config");
        write_file(&dir.join(AGENT_CONFIG_FILE), "not: valid: yaml: [");
        let src = source(&dir);
        assert!(src.read_tie_off_instruction().is_none());
        assert!(src.read_event_content_instruction().is_none());
    }
}
