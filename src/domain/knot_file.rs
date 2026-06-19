use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::domain::value_objects::{AgentProfile, PromptTemplate};

pub use crate::domain::value_objects::AgentProfileError;

// Re-export AgentProfileError at the knot_file level for convenient access.

// ── Errors ─────────────────────────────────────────────────────────────────

/// Errors produced when parsing or validating a knot definition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnotFileError {
    /// The `name` field is missing from frontmatter.
    MissingName,
    /// The `prompt-template` section is missing from frontmatter.
    MissingPromptTemplate,
    /// The `strand-dir` field is missing or empty.
    MissingStrandDir,
    /// The frontmatter YAML could not be parsed.
    InvalidFormat,
    /// The `agent-profile-ref` field is missing.
    MissingProfileRef,
}

impl std::fmt::Display for KnotFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KnotFileError::MissingName => {
                write!(f, "knot file is missing 'name' field")
            }
            KnotFileError::MissingPromptTemplate => {
                write!(f, "knot file is missing 'prompt-template' section")
            }
            KnotFileError::MissingStrandDir => {
                write!(f, "knot file is missing 'strand-dir' field")
            }
            KnotFileError::InvalidFormat => {
                write!(f, "knot file frontmatter is not valid YAML")
            }
            KnotFileError::MissingProfileRef => {
                write!(f, "knot file must have 'agent-profile-ref'")
            }
        }
    }
}

impl std::error::Error for KnotFileError {}

// ── Knot File ──────────────────────────────────────────────────────────────

/// Parsed representation of a knot definition file.
///
/// A knot file is markdown with YAML frontmatter delimited by `---`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnotFile {
    /// The name of the knot (becomes the `KnotId`).
    pub name: String,
    /// Reference to a named agent profile stored in `profiles/{name}.md`.
    pub agent_profile_ref: String,
    /// Prompt template extracted from frontmatter.
    pub prompt_template: PromptTemplate,
    /// Directory to watch for strand files (required).
    pub strand_dir: PathBuf,
    /// When `true` (default), a git commit is created after each successful
    /// knot run. Parsed from `git-versioned` frontmatter key.
    pub git_versioned: bool,
}

/// Internal YAML structure for frontmatter parsing.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    #[serde(rename = "agent-profile-ref")]
    agent_profile_ref: Option<String>,
    #[serde(rename = "prompt-template")]
    prompt_template: Option<RawPromptTemplate>,
    #[serde(rename = "strand-dir")]
    strand_dir: Option<String>,
    #[serde(rename = "git-versioned")]
    git_versioned: Option<bool>,
    /// Captures any unknown YAML keys for warning emission.
    #[allow(dead_code)]
    #[serde(flatten)]
    extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
struct RawPromptTemplate {
    instructions: Option<String>,
}

/// Parse a knot definition file from its string content.
///
/// Extracts and validates the YAML frontmatter. The body (markdown after the
/// closing `---`) is not parsed — it is documentation only.
///
/// Required fields: `name`, `agent-profile-ref`, `prompt-template`, `strand-dir`.
///
/// Returns a tuple of `(KnotFile, Vec<String>)` where the second element
/// contains warning messages for each unknown YAML property found in the
/// frontmatter. Unknown properties do not cause parse failures.
pub fn parse(
    content: &str,
) -> Result<(KnotFile, Vec<String>), KnotFileError> {
    // Split on frontmatter delimiters
    let yaml_text = extract_frontmatter(content)?;
    let raw: RawFrontmatter =
        serde_yaml::from_str(&yaml_text).map_err(|_| KnotFileError::InvalidFormat)?;

    // Collect warnings for unknown properties.
    let warnings: Vec<String> = raw
        .extra
        .keys()
        .map(|key| format!("unknown property '{key}' in knot frontmatter (not used)"))
        .collect();

    // Validate name
    let name = raw
        .name
        .filter(|n| !n.trim().is_empty())
        .ok_or(KnotFileError::MissingName)?;

    // agent-profile-ref is required
    let agent_profile_ref = raw
        .agent_profile_ref
        .filter(|r| !r.trim().is_empty())
        .ok_or(KnotFileError::MissingProfileRef)?;

    // Validate prompt-template
    let raw_template = raw
        .prompt_template
        .ok_or(KnotFileError::MissingPromptTemplate)?;

    let instructions = raw_template
        .instructions
        .filter(|ins| !ins.trim().is_empty())
        .ok_or(KnotFileError::MissingPromptTemplate)?;

    let prompt_template = PromptTemplate::new(instructions)
        .map_err(|_| KnotFileError::MissingPromptTemplate)?;

    // Parse required strand-dir
    let strand_dir = raw
        .strand_dir
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .ok_or(KnotFileError::MissingStrandDir)?;

    // git-versioned defaults to true when absent
    let git_versioned = raw.git_versioned.unwrap_or(true);

    Ok((
        KnotFile {
            name,
            agent_profile_ref,
            prompt_template,
            strand_dir,
            git_versioned,
        },
        warnings,
    ))
}

/// Derive the tie-off output directory for a knot.
///
/// Returns `rig/tie-offs/{loom-id}/{knot-name}/`. Individual strand
/// tie-off files (e.g. `{knot-name}-tie-off.md`) are placed inside this
/// directory by `ProcessStrand`.
pub fn derive_tieoff_path(
    loom_id: &str,
    knot_name: &str,
    rig: &std::path::Path,
) -> std::path::PathBuf {
    rig.join("tie-offs").join(loom_id).join(knot_name)
}

/// Derive the loom-log path for a loom.
///
/// Returns `rig/tie-offs/{loom-id}/.loom-log`.
/// Moved from `rig/{loom-id}/.loom-log` to separate outputs from definitions.
pub fn derive_loom_log_path(
    loom_id: &str,
    rig: &std::path::Path,
) -> std::path::PathBuf {
    rig.join("tie-offs").join(loom_id).join(".loom-log")
}

/// Extract the YAML portion between the first pair of `---` delimiters.
fn extract_frontmatter(content: &str) -> Result<String, KnotFileError> {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        return Err(KnotFileError::InvalidFormat);
    }

    // Find the closing `---`
    let rest = &trimmed[3..]; // skip opening `---`
    let closing_pos = rest
        .find("---")
        .ok_or(KnotFileError::InvalidFormat)?;

    Ok(rest[..closing_pos].trim().to_string())
}

// ── Agent Profile Parsing ──────────────────────────────────────────────────

/// Internal YAML structure for agent profile frontmatter parsing.
#[derive(Debug, Deserialize)]
struct RawProfileFrontmatter {
    name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    #[serde(rename = "profile-prompt")]
    profile_prompt: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    timeout: Option<u64>,
}

/// Parse an agent profile file from its string content.
///
/// Extracts and validates the YAML frontmatter. The body (markdown after the
/// closing `---`) is not parsed — it is documentation only.
///
/// Required fields: `name`, `provider`, `model`, `profile-prompt`.
/// Optional field: `tools`.
pub fn parse_agent_profile(
    content: &str,
) -> Result<AgentProfile, AgentProfileError> {
    // Use shared frontmatter extraction (same logic as KnotFile::parse).
    let yaml_text =
        extract_frontmatter(content).map_err(|_| AgentProfileError::InvalidFormat)?;
    let raw: RawProfileFrontmatter = serde_yaml::from_str(&yaml_text).map_err(|_| {
        AgentProfileError::InvalidFormat
    })?;

    // Validate name
    let name = raw
        .name
        .filter(|n| !n.trim().is_empty())
        .ok_or(AgentProfileError::MissingName)?;

    // Validate provider
    let provider = raw
        .provider
        .filter(|p| !p.trim().is_empty())
        .ok_or(AgentProfileError::EmptyProvider)?;

    // Validate model
    let model = raw
        .model
        .filter(|m| !m.trim().is_empty())
        .ok_or(AgentProfileError::EmptyModel)?;

    // Validate profile-prompt
    let profile_prompt = raw
        .profile_prompt
        .filter(|s| !s.trim().is_empty())
        .ok_or(AgentProfileError::MissingProfilePrompt)?;

    // Build profile with optional tools and timeout
    AgentProfile::with_tools(
        name,
        provider,
        model,
        raw.tools.unwrap_or_default(),
        profile_prompt,
    )
    .map(|p| p.with_timeout(raw.timeout))
}



// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const VALID_KNOT: &str = "---
name: prd-goals-review
agent-profile-ref: fast
strand-dir: \"strands\"
prompt-template:
  instructions: |
    Review the goals section of this PRD. Check that:
    - Each goal is specific and measurable
    - Goals align with the problem statement
---

# PRD Goals Review Knot

This knot reviews the goals section of PRD documents.
";

    #[test]
    fn valid_knot_file_parse() {
        let (file, warnings) = parse(VALID_KNOT).unwrap();
        assert!(
            warnings.is_empty(),
            "valid knot should produce no warnings, got: {warnings:?}"
        );
        assert_eq!(file.name, "prd-goals-review");
        assert_eq!(file.agent_profile_ref, "fast");
        assert!(file.prompt_template.instructions.contains("specific and measurable"));
        assert_eq!(file.strand_dir, PathBuf::from("strands"));
    }

    #[test]
    fn knot_file_with_custom_strand_dir() {
        let content = "---
name: custom-dirs-knot
agent-profile-ref: fast
strand-dir: \"../custom-source\"
prompt-template:
  instructions: \"Review the document\"
---

Body.
";

        let (file, warnings) = parse(content).unwrap();
        assert!(
            warnings.is_empty(),
            "custom-strand-dir knot should produce no warnings, got: {warnings:?}"
        );
        assert_eq!(file.name, "custom-dirs-knot");
        assert_eq!(
            file.strand_dir,
            PathBuf::from("../custom-source")
        );
    }

    #[test]
    fn missing_strand_dir_returns_error() {
        let content = "---
name: no-strand-dir-knot
agent-profile-ref: fast
prompt-template:
  instructions: \"Review the document\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::MissingStrandDir);
    }

    #[test]
    fn unknown_property_emits_warning() {
        // Unknown YAML properties (including formerly-accepted tie-off-dir)
        // now emit warnings.
        let content = "---
name: legacy-knot
agent-profile-ref: fast
strand-dir: \"../input\"
tie-off-dir: \"../old-output\"
prompt-template:
  instructions: \"Review the document\"
---

Body.
";

        let (file, warnings) = parse(content).unwrap();
        assert_eq!(file.name, "legacy-knot");
        assert_eq!(file.strand_dir, PathBuf::from("../input"));
        assert_eq!(file.agent_profile_ref, "fast");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("tie-off-dir"));
    }

    #[test]
    fn parse_detects_unknown_properties() {
        let content = "---
name: unknown-props-knot
agent-profile-ref: fast
strand-dir: \"strands\"
foo: bar
baz: 42
prompt-template:
  instructions: \"Review the document\"
---

Body.
";

        let (file, warnings) = parse(content).unwrap();
        assert_eq!(file.name, "unknown-props-knot");
        assert_eq!(file.agent_profile_ref, "fast");
        assert_eq!(warnings.len(), 2);
        assert!(
            warnings.iter().any(|w| w.contains("foo")),
            "warning should mention 'foo'"
        );
        assert!(
            warnings.iter().any(|w| w.contains("baz")),
            "warning should mention 'baz'"
        );
    }

    #[test]
    fn parse_no_warnings_for_valid_knot() {
        let content = "---
name: clean-knot
agent-profile-ref: fast
strand-dir: \"strands\"
prompt-template:
  instructions: \"Review the document\"
---

Body.
";

        let (file, warnings) = parse(content).unwrap();
        assert_eq!(file.name, "clean-knot");
        assert!(
            warnings.is_empty(),
            "valid knot should produce no warnings, got: {warnings:?}"
        );
    }

    #[test]
    fn empty_strand_dir_returns_error() {
        let content = "---
name: empty-strand-knot
agent-profile-ref: fast
strand-dir: \"  \"
prompt-template:
  instructions: \"Review the document\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::MissingStrandDir);
    }

    #[test]
    fn knot_file_error_display() {
        assert_eq!(
            KnotFileError::MissingName.to_string(),
            "knot file is missing 'name' field"
        );
        assert_eq!(
            KnotFileError::MissingPromptTemplate.to_string(),
            "knot file is missing 'prompt-template' section"
        );
        assert_eq!(
            KnotFileError::MissingStrandDir.to_string(),
            "knot file is missing 'strand-dir' field"
        );
        assert_eq!(
            KnotFileError::InvalidFormat.to_string(),
            "knot file frontmatter is not valid YAML"
        );
        assert_eq!(
            KnotFileError::MissingProfileRef.to_string(),
            "knot file must have 'agent-profile-ref'"
        );
    }

    #[test]
    fn derive_tieoff_path_builds_correct_path() {
        let path = derive_tieoff_path("my-loom", "review-knot", Path::new("/workspace/rig"));
        assert_eq!(
            path,
            PathBuf::from("/workspace/rig/tie-offs/my-loom/review-knot")
        );
    }

    #[test]
    fn derive_loom_log_path_builds_correct_path() {
        let path = derive_loom_log_path("my-loom", Path::new("/workspace/rig"));
        assert_eq!(
            path,
            PathBuf::from("/workspace/rig/tie-offs/my-loom/.loom-log")
        );
    }

    // ── Profile Ref Validation Tests ──────────────────────────────────────

    #[test]
    fn knot_with_profile_ref_parses() {
        let (file, warnings) = parse(VALID_KNOT).unwrap();
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {warnings:?}"
        );
        assert_eq!(file.name, "prd-goals-review");
        assert_eq!(file.agent_profile_ref, "fast");
    }

    #[test]
    fn missing_profile_ref_returns_error() {
        let content = r#"---
name: no-ref-knot
strand-dir: "strands"
prompt-template:
  instructions: "Do something"
---

Body.
"#;

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KnotFileError::MissingProfileRef
        );
    }

    #[test]
    fn empty_profile_ref_returns_error() {
        let content = r#"---
name: empty-ref-knot
agent-profile-ref: "  "
strand-dir: "strands"
prompt-template:
  instructions: "Do something"
---

Body.
"#;

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KnotFileError::MissingProfileRef
        );
    }

    #[test]
    fn knot_serialization_with_profile_ref() {
        let file = KnotFile {
            name: "test".to_string(),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate::new("do it".to_string())
                .unwrap(),
            strand_dir: PathBuf::from("strands/test"),
            git_versioned: true,
        };

        let json = serde_json::to_string(&file).unwrap();
        let deserialized: KnotFile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, file);
    }

    #[test]
    fn roundtrip_generate_and_parse() {
        // generate_knot_file produces content that KnotFile::parse can read
        let content = "---\nname: roundtrip-knot\nagent-profile-ref: fast\nstrand-dir: \"strands\"\nprompt-template:\n  instructions: \"Process this\"\n---\n\n# roundtrip-knot\n\nRoundtrip test.\n";
        let (file, warnings) = parse(content).unwrap();
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {warnings:?}"
        );
        assert_eq!(file.name, "roundtrip-knot");
        assert_eq!(file.agent_profile_ref, "fast");
        assert_eq!(file.strand_dir, PathBuf::from("strands"));
        assert!(file.git_versioned, "should default to true");
    }

    // ── Git Versioned Tests ──────────────────────────────────────────────

    #[test]
    fn knot_file_with_git_versioned_true() {
        let content = "---\nname: git-on-knot\nagent-profile-ref: fast\nstrand-dir: \"strands\"\ngit-versioned: true\nprompt-template:\n  instructions: \"Do it\"\n---\n\nBody.\n";
        let (file, warnings) = parse(content).unwrap();
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {warnings:?}"
        );
        assert_eq!(file.name, "git-on-knot");
        assert!(file.git_versioned);
    }

    #[test]
    fn knot_file_with_git_versioned_false() {
        let content = "---\nname: git-off-knot\nagent-profile-ref: fast\nstrand-dir: \"strands\"\ngit-versioned: false\nprompt-template:\n  instructions: \"Do it\"\n---\n\nBody.\n";
        let (file, warnings) = parse(content).unwrap();
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {warnings:?}"
        );
        assert_eq!(file.name, "git-off-knot");
        assert!(!file.git_versioned);
    }

    #[test]
    fn knot_file_without_git_versioned_defaults_true() {
        let content = "---\nname: default-git-knot\nagent-profile-ref: fast\nstrand-dir: \"strands\"\nprompt-template:\n  instructions: \"Do it\"\n---\n\nBody.\n";
        let (file, warnings) = parse(content).unwrap();
        assert!(
            warnings.is_empty(),
            "no warnings expected, got: {warnings:?}"
        );
        assert_eq!(file.name, "default-git-knot");
        assert!(
            file.git_versioned,
            "git_versioned should default to true when absent"
        );
    }

    #[test]
    fn knot_file_roundtrip_with_git_versioned() {
        // Parse a knot file with git-versioned: false, then serialize and
        // parse again to verify the value survives round-trip.
        let content = "---\nname: roundtrip-git-knot\nagent-profile-ref: fast\nstrand-dir: \"strands\"\ngit-versioned: false\nprompt-template:\n  instructions: \"Do it\"\n---\n\nBody.\n";
        let (file, _warnings) = parse(content).unwrap();
        assert!(!file.git_versioned);

        // Serialize back to JSON and deserialize
        let json = serde_json::to_string(&file).unwrap();
        let deserialized: KnotFile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, file);
        assert!(!deserialized.git_versioned);
    }

    // ── Agent Profile Parsing Tests ──────────────────────────────────────────

    const VALID_PROFILE: &str = "---
name: fast
provider: openai
model: gpt-4o
tools:
  - fs
profile-prompt: |
  You are a fast reviewer. Keep responses concise and direct.
---

# Fast Profile

Lightweight profile for quick reviews.
";

    #[test]
    fn parse_valid_agent_profile() {
        let result = parse_agent_profile(VALID_PROFILE);
        assert!(result.is_ok(), "valid profile should parse without error");

        let profile = result.unwrap();
        assert_eq!(profile.name, "fast");
        assert_eq!(profile.provider, "openai");
        assert_eq!(profile.model, "gpt-4o");
        assert_eq!(profile.tools, vec!["fs"]);
        assert!(profile.profile_prompt.contains("fast reviewer"));
    }

    #[test]
    fn parse_profile_without_tools() {
        let content = "---
name: minimal
provider: anthropic
model: claude-sonnet-4-20250514
profile-prompt: Review the document.
---

Body.
";
        let profile = parse_agent_profile(content).unwrap();
        assert_eq!(profile.name, "minimal");
        assert_eq!(profile.provider, "anthropic");
        assert_eq!(profile.model, "claude-sonnet-4-20250514");
        assert!(profile.tools.is_empty());
        assert_eq!(profile.profile_prompt, "Review the document.");
    }

    #[test]
    fn parse_profile_with_multiline_profile_prompt() {
        let content = "---
name: detailed
provider: openai
model: gpt-4o
profile-prompt: |
  You are a detailed reviewer.

  Keep responses thorough.
---

Body.
";
        let profile = parse_agent_profile(content).unwrap();
        assert!(profile.profile_prompt.contains("detailed reviewer"));
        assert!(profile.profile_prompt.contains("thorough"));
    }

    #[test]
    fn parse_profile_missing_name() {
        let content = "---
provider: openai
model: gpt-4o
profile-prompt: Review.
---

Body.
";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::MissingName);
    }

    #[test]
    fn parse_profile_empty_name() {
        let content = "---\nname: \nprovider: openai\nmodel: gpt-4o\nprofile-prompt: Review.\n---\n\nBody.\n";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::MissingName);
    }

    #[test]
    fn parse_profile_missing_provider() {
        let content = "---
name: test
model: gpt-4o
profile-prompt: Review.
---

Body.
";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::EmptyProvider);
    }

    #[test]
    fn parse_profile_missing_model() {
        let content = "---
name: test
provider: openai
profile-prompt: Review.
---

Body.
";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::EmptyModel);
    }

    #[test]
    fn parse_profile_missing_profile_prompt() {
        let content = "---
name: test
provider: openai
model: gpt-4o
---

Body.
";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AgentProfileError::MissingProfilePrompt
        );
    }

    #[test]
    fn parse_profile_empty_profile_prompt() {
        let content = "---\nname: test\nprovider: openai\nmodel: gpt-4o\nprofile-prompt: \n---\n\nBody.\n";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AgentProfileError::MissingProfilePrompt
        );
    }

    #[test]
    fn parse_profile_whitespace_fields() {
        let content = "---\nname:    \nprovider:    \nmodel:    \nprofile-prompt:      \n---\n\nBody.\n".to_string();
        let result = parse_agent_profile(&content);
        assert!(result.is_err());
        // name is checked first
        assert_eq!(result.unwrap_err(), AgentProfileError::MissingName);
    }

    #[test]
    fn parse_profile_no_frontmatter() {
        let content = "# Just a markdown file\n\nNo frontmatter.";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::InvalidFormat);
    }

    #[test]
    fn parse_profile_no_closing_delimiter() {
        let content = "---
name: test
provider: openai";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::InvalidFormat);
    }

    #[test]
    fn parse_profile_malformed_yaml() {
        let content = "---
name: test
  broken: yaml: [
provider: openai
model: gpt-4o
profile-prompt: Review.
---

Body.
";
        let result = parse_agent_profile(content);
        assert!(result.is_err());
    }

    #[test]
    fn agent_profile_error_display() {
        assert_eq!(
            AgentProfileError::MissingName.to_string(),
            "agent profile must have a name"
        );
        assert_eq!(
            AgentProfileError::EmptyProvider.to_string(),
            "agent profile provider must not be empty"
        );
        assert_eq!(
            AgentProfileError::EmptyModel.to_string(),
            "agent profile model must not be empty"
        );
        assert_eq!(
            AgentProfileError::MissingProfilePrompt.to_string(),
            "agent profile profile_prompt must not be empty"
        );
        assert_eq!(
            AgentProfileError::InvalidFormat.to_string(),
            "agent profile file has no valid frontmatter"
        );
    }

    #[test]
    fn parse_profile_with_multiple_tools() {
        let content = "---
name: full-stack
provider: openai
model: gpt-4o
tools:
  - fs
  - web
  - sql
profile-prompt: Full stack review.
---

Body.
";
        let profile = parse_agent_profile(content).unwrap();
        assert_eq!(profile.tools, vec!["fs", "web", "sql"]);
    }

    #[test]
    fn parse_profile_with_timeout() {
        let content = "---
name: slow-model
provider: anthropic
model: claude-sonnet-4-20250514
timeout: 600
profile-prompt: Thorough review with long timeout.
---

Body.
";
        let profile = parse_agent_profile(content).unwrap();
        assert_eq!(profile.name, "slow-model");
        assert_eq!(profile.timeout, Some(600));
    }

    #[test]
    fn parse_profile_without_timeout() {
        let content = "---
name: fast
provider: openai
model: gpt-4o
profile-prompt: Quick review.
---

Body.
";
        let profile = parse_agent_profile(content).unwrap();
        assert_eq!(profile.timeout, None);
    }

    #[test]
    fn parse_profile_with_timeout_and_tools() {
        let content = "---
name: full-timed
provider: anthropic
model: claude-sonnet
tools:
  - fs
  - web
timeout: 300
profile-prompt: Full review with timeout.
---

Body.
";
        let profile = parse_agent_profile(content).unwrap();
        assert_eq!(profile.timeout, Some(300));
        assert_eq!(profile.tools, vec!["fs", "web"]);
    }
}
