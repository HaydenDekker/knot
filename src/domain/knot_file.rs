use serde::{Deserialize, Serialize};

use crate::domain::value_objects::{AgentConfig, PromptTemplate};

// ── Errors ─────────────────────────────────────────────────────────────────

/// Errors produced when parsing or validating a knot definition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnotFileError {
    /// The `name` field is missing from frontmatter.
    MissingName,
    /// The `goal` field is empty or whitespace-only.
    EmptyGoal,
    /// The `prompt-template` section is missing from frontmatter.
    MissingPromptTemplate,
    /// The frontmatter YAML could not be parsed.
    InvalidFormat,
}

impl std::fmt::Display for KnotFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KnotFileError::MissingName => write!(f, "knot file is missing 'name' field"),
            KnotFileError::EmptyGoal => write!(f, "knot file 'goal' field is empty"),
            KnotFileError::MissingPromptTemplate => {
                write!(f, "knot file is missing 'prompt-template' section")
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
}

/// Internal YAML structure for frontmatter parsing.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    #[serde(rename = "agent-config")]
    agent_config: Option<RawAgentConfig>,
    #[serde(rename = "prompt-template")]
    prompt_template: Option<RawPromptTemplate>,
}

#[derive(Debug, Deserialize)]
struct RawAgentConfig {
    goal: Option<String>,
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

    // Validate agent-config.goal
    let goal = raw
        .agent_config
        .and_then(|ac| ac.goal)
        .filter(|g| !g.trim().is_empty())
        .ok_or(KnotFileError::EmptyGoal)?;
    let agent_config = AgentConfig::new(goal)
        .map_err(|_| KnotFileError::EmptyGoal)?;

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

    Ok(KnotFile {
        name,
        agent_config,
        prompt_template,
    })
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

    const VALID_KNOT: &str = "---
name: prd-goals-review
agent-config:
  goal: \"Review PRD goals for clarity, completeness, and alignment\"
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
        assert_eq!(file.prompt_template.input_bundling, "full-file");
        assert!(file.prompt_template.instructions.contains("specific and measurable"));
    }

    #[test]
    fn missing_name_returns_error() {
        let content = "---
agent-config:
  goal: \"Some goal\"
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
            agent_config: AgentConfig::new("test goal".to_string()).unwrap(),
            prompt_template: PromptTemplate::new(
                "full-file".to_string(),
                "do it".to_string(),
            )
            .unwrap(),
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
            KnotFileError::MissingPromptTemplate.to_string(),
            "knot file is missing 'prompt-template' section"
        );
        assert_eq!(
            KnotFileError::InvalidFormat.to_string(),
            "knot file frontmatter is not valid YAML"
        );
    }
}
