use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::value_objects::{AgentConfig, PromptTemplate};

// ── Errors ─────────────────────────────────────────────────────────────────

/// Errors produced when parsing or validating a knot definition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnotFileError {
    /// The `name` field is missing from frontmatter.
    MissingName,
    /// The `goal` field is empty or whitespace-only.
    EmptyGoal,
    /// The `provider` field is empty or whitespace-only.
    EmptyProvider,
    /// The `model` field is empty or whitespace-only.
    EmptyModel,
    /// The `prompt-template` section is missing from frontmatter.
    MissingPromptTemplate,
    /// The `strand-dir` field is missing or empty.
    MissingStrandDir,
    /// The frontmatter YAML could not be parsed.
    InvalidFormat,
}

impl std::fmt::Display for KnotFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KnotFileError::MissingName => {
                write!(f, "knot file is missing 'name' field")
            }
            KnotFileError::EmptyGoal => {
                write!(f, "knot file 'goal' field is empty")
            }
            KnotFileError::EmptyProvider => {
                write!(f, "knot file 'provider' field is empty")
            }
            KnotFileError::EmptyModel => {
                write!(f, "knot file 'model' field is empty")
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
    /// Agent configuration extracted from frontmatter.
    pub agent_config: AgentConfig,
    /// Prompt template extracted from frontmatter.
    pub prompt_template: PromptTemplate,
    /// Directory to watch for strand files (required).
    pub strand_dir: PathBuf,
}

/// Internal YAML structure for frontmatter parsing.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    #[serde(rename = "agent-config")]
    agent_config: Option<RawAgentConfig>,
    #[serde(rename = "prompt-template")]
    prompt_template: Option<RawPromptTemplate>,
    #[serde(rename = "strand-dir")]
    strand_dir: Option<String>,
    /// Tie-off dir is accepted in YAML but ignored (output path is now static).
    #[allow(dead_code)]
    #[serde(rename = "tie-off-dir")]
    tie_off_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAgentConfig {
    goal: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawPromptTemplate {
    #[serde(rename = "input-bundling")]
    input_bundling: Option<String>,
    instructions: Option<String>,
}

/// Parse a knot definition file from its string content.
///
/// Extracts and validates the YAML frontmatter. The body (markdown after the
/// closing `---`) is not parsed — it is documentation only.
pub fn parse(content: &str) -> Result<KnotFile, KnotFileError> {
    // Split on frontmatter delimiters
    let yaml_text = extract_frontmatter(content)?;
    let raw: RawFrontmatter =
        serde_yaml::from_str(&yaml_text).map_err(|_| KnotFileError::InvalidFormat)?;

    // Validate name
    let name = raw
        .name
        .filter(|n| !n.trim().is_empty())
        .ok_or(KnotFileError::MissingName)?;

    // Validate agent-config fields
    let ac = raw
        .agent_config
        .as_ref()
        .ok_or(KnotFileError::EmptyGoal)?;
    let goal = ac
        .goal
        .as_ref()
        .filter(|g| !g.trim().is_empty())
        .ok_or(KnotFileError::EmptyGoal)?;
    let provider = ac
        .provider
        .as_ref()
        .filter(|p| !p.trim().is_empty())
        .ok_or(KnotFileError::EmptyProvider)?;
    let model = ac
        .model
        .as_ref()
        .filter(|m| !m.trim().is_empty())
        .ok_or(KnotFileError::EmptyModel)?;

    let mut agent_config = AgentConfig::new(
        goal.clone(),
        provider.clone(),
        model.clone(),
    )
    .map_err(|_| KnotFileError::EmptyGoal)?;
    // Apply optional tools
    if let Some(tools) = &ac.tools {
        agent_config.tools = tools.clone();
    }

    // Validate prompt-template
    let raw_template = raw
        .prompt_template
        .ok_or(KnotFileError::MissingPromptTemplate)?;

    let input_bundling = raw_template
        .input_bundling
        .filter(|ib| !ib.trim().is_empty())
        .ok_or(KnotFileError::MissingPromptTemplate)?;
    let instructions = raw_template
        .instructions
        .filter(|ins| !ins.trim().is_empty())
        .ok_or(KnotFileError::MissingPromptTemplate)?;

    let prompt_template = PromptTemplate::new(input_bundling, instructions)
        .map_err(|_| KnotFileError::MissingPromptTemplate)?;

    // Parse required strand-dir
    let strand_dir = raw
        .strand_dir
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .ok_or(KnotFileError::MissingStrandDir)?;

    Ok(KnotFile {
        name,
        agent_config,
        prompt_template,
        strand_dir,
    })
}

/// Derive the tie-off output path for a knot.
///
/// Returns `rig/output/{loom-id}/{knot-name}/output.md`.
/// This is the statically-derived replacement for the per-knot
/// Derive the tie-off output directory for a knot.
///
/// Returns `rig/output/{loom-id}/{knot-name}/`.
/// Each strand's output file (e.g., `strand.md.output`) is placed
/// inside this directory.
pub fn derive_tieoff_path(
    loom_id: &str,
    knot_name: &str,
    rig: &std::path::Path,
) -> std::path::PathBuf {
    rig.join("output").join(loom_id).join(knot_name)
}

/// Derive the loom-log path for a loom.
///
/// Returns `rig/output/{loom-id}/.loom-log`.
/// Moved from `rig/{loom-id}/.loom-log` to separate outputs from definitions.
pub fn derive_loom_log_path(
    loom_id: &str,
    rig: &std::path::Path,
) -> std::path::PathBuf {
    rig.join("output").join(loom_id).join(".loom-log")
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const VALID_KNOT: &str = "---
name: prd-goals-review
agent-config:
  goal: \"Review PRD goals for clarity, completeness, and alignment\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"strands\"
prompt-template:
  input-bundling: \"full-file\"
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
        let result = parse(VALID_KNOT);
        assert!(result.is_ok(), "valid knot should parse without error");

        let file = result.unwrap();
        assert_eq!(file.name, "prd-goals-review");
        assert_eq!(file.agent_config.goal, "Review PRD goals for clarity, completeness, and alignment");
        assert_eq!(file.agent_config.provider, "openai");
        assert_eq!(file.agent_config.model, "gpt-4o");
        assert!(file.agent_config.tools.is_empty());
        assert_eq!(file.prompt_template.input_bundling, "full-file");
        assert!(file.prompt_template.instructions.contains("specific and measurable"));
        assert_eq!(file.strand_dir, PathBuf::from("strands"));
    }

    #[test]
    fn knot_file_with_custom_strand_dir() {
        let content = "---
name: custom-dirs-knot
agent-config:
  goal: \"Review with custom dirs\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"../custom-source\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review the document\"
---

Body.
";

        let file = parse(content).unwrap();
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
agent-config:
  goal: \"Review\"
  provider: \"openai\"
  model: \"gpt-4o\"
tie-off-dir: \"../output\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review the document\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::MissingStrandDir);
    }

    #[test]
    fn tieoff_dir_in_yaml_is_accepted_but_ignored() {
        // tie-off-dir in YAML frontmatter is accepted (for backward compat)
        // but is not stored — output path is statically derived.
        let content = "---
name: legacy-knot
agent-config:
  goal: \"Review\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"../input\"
tie-off-dir: \"../old-output\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review the document\"
---

Body.
";

        let file = parse(content).unwrap();
        assert_eq!(file.name, "legacy-knot");
        assert_eq!(file.strand_dir, PathBuf::from("../input"));
    }

    #[test]
    fn empty_strand_dir_returns_error() {
        let content = "---
name: empty-strand-knot
agent-config:
  goal: \"Review\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"  \"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review the document\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::MissingStrandDir);
    }

    #[test]
    fn knot_file_with_provider_model_tools() {
        let content = "---
name: review-knot
agent-config:
  goal: \"Review document\"
  provider: \"anthropic\"
  model: \"claude-sonnet-4-20250514\"
  tools:
    - fs
    - web
strand-dir: \"strands\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review the document\"
---

Body.
";

        let file = parse(content).unwrap();
        assert_eq!(file.name, "review-knot");
        assert_eq!(file.agent_config.provider, "anthropic");
        assert_eq!(file.agent_config.model, "claude-sonnet-4-20250514");
        assert_eq!(file.agent_config.tools, vec!["fs", "web"]);
    }

    #[test]
    fn missing_name_returns_error() {
        let content = "---
agent-config:
  goal: \"Some goal\"
  provider: \"openai\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

No name here.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::MissingName);
    }

    #[test]
    fn empty_goal_returns_error() {
        let content = "---
name: test-knot
agent-config:
  goal: \"\"
  provider: \"openai\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::EmptyGoal);
    }

    #[test]
    fn missing_prompt_template_returns_error() {
        let content = "---
name: test-knot
agent-config:
  goal: \"Some goal\"
  provider: \"openai\"
  model: \"gpt-4o\"
---

No prompt template.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            KnotFileError::MissingPromptTemplate
        );
    }

    #[test]
    fn malformed_yaml_returns_error() {
        let content = "---
name: test-knot
  broken: yaml: [
agent-config:
  goal: \"Some goal\"
  provider: \"openai\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::InvalidFormat);
    }

    #[test]
    fn no_frontmatter_returns_error() {
        let content = "# Just a markdown file

No frontmatter at all.";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::InvalidFormat);
    }

    #[test]
    fn knot_file_serialization() {
        let file = KnotFile {
            name: "test".to_string(),
            agent_config: AgentConfig::new(
                "test goal".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
            )
            .unwrap(),
            prompt_template: PromptTemplate::new(
                "full-file".to_string(),
                "do it".to_string(),
            )
            .unwrap(),
            strand_dir: PathBuf::from("strands/test"),
        };

        let json = serde_json::to_string(&file).unwrap();
        let deserialized: KnotFile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, file);
    }

    #[test]
    fn whitespace_only_goal_returns_error() {
        let content = "---
name: test-knot
agent-config:
  goal: \"   \"
  provider: \"openai\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::EmptyGoal);
    }

    #[test]
    fn missing_agent_config_returns_error() {
        let content = "---
name: test-knot
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::EmptyGoal);
    }

    #[test]
    fn missing_provider_returns_error() {
        let content = "---
name: test-knot
agent-config:
  goal: \"Some goal\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::EmptyProvider);
    }

    #[test]
    fn missing_model_returns_error() {
        let content = "---
name: test-knot
agent-config:
  goal: \"Some goal\"
  provider: \"openai\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Do something\"
---

Body.
";

        let result = parse(content);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), KnotFileError::EmptyModel);
    }

    #[test]
    fn knot_file_error_display() {
        assert_eq!(
            KnotFileError::MissingName.to_string(),
            "knot file is missing 'name' field"
        );
        assert_eq!(
            KnotFileError::EmptyGoal.to_string(),
            "knot file 'goal' field is empty"
        );
        assert_eq!(
            KnotFileError::EmptyProvider.to_string(),
            "knot file 'provider' field is empty"
        );
        assert_eq!(
            KnotFileError::EmptyModel.to_string(),
            "knot file 'model' field is empty"
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
            KnotFileError::MissingStrandDir.to_string(),
            "knot file is missing 'strand-dir' field"
        );
    }

    #[test]
    fn derive_tieoff_path_builds_correct_path() {
        let path = derive_tieoff_path("my-loom", "review-knot", Path::new("/workspace/rig"));
        assert_eq!(
            path,
            PathBuf::from("/workspace/rig/output/my-loom/review-knot")
        );
    }

    #[test]
    fn derive_loom_log_path_builds_correct_path() {
        let path = derive_loom_log_path("my-loom", Path::new("/workspace/rig"));
        assert_eq!(
            path,
            PathBuf::from("/workspace/rig/output/my-loom/.loom-log")
        );
    }
}
