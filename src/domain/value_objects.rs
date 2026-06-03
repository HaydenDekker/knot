use serde::{Deserialize, Serialize};

// ── Errors ─────────────────────────────────────────────────────────────────

/// Domain-level validation errors for value objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// A required field was empty or missing.
    EmptyField(String),
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyField(field) => write!(f, "field '{field}' must not be empty"),
        }
    }
}

impl std::error::Error for DomainError {}

// ── Value Objects ──────────────────────────────────────────────────────────

/// Configuration for the agent that runs a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// The goal this knot's agent should accomplish.
    pub goal: String,
}

impl AgentConfig {
    /// Create a new `AgentConfig` with a non-empty goal.
    ///
    /// Returns `DomainError::EmptyField` if the goal is blank.
    pub fn new(goal: String) -> Result<Self, DomainError> {
        if goal.trim().is_empty() {
            return Err(DomainError::EmptyField("goal".to_string()));
        }
        Ok(Self { goal })
    }
}

/// Prompt template used when executing a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// How input is bundled: e.g. "full-file", "diff", "chunked".
    pub input_bundling: String,
    /// The prompt instructions.
    pub instructions: String,
}

impl PromptTemplate {
    /// Create a new `PromptTemplate` with non-empty fields.
    ///
    /// Returns `DomainError::EmptyField` for any blank field.
    pub fn new(
        input_bundling: String,
        instructions: String,
    ) -> Result<Self, DomainError> {
        if input_bundling.trim().is_empty() {
            return Err(DomainError::EmptyField(
                "input_bundling".to_string(),
            ));
        }
        if instructions.trim().is_empty() {
            return Err(DomainError::EmptyField(
                "instructions".to_string(),
            ));
        }
        Ok(Self {
            input_bundling,
            instructions,
        })
    }
}

/// Workspace-level agent configuration. One config per workspace,
/// shared by all knots in that workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceAgentConfig {
    /// Path to the agent CLI binary.
    pub cli_path: String,
    /// Arguments passed to the CLI.
    pub cli_args: Vec<String>,
}

impl WorkspaceAgentConfig {
    /// Create a default workspace config (`cli_path = "pi"`, `cli_args = []`).
    pub fn default_config() -> Self {
        Self {
            cli_path: "pi".to_string(),
            cli_args: Vec::new(),
        }
    }

    /// Create a custom workspace config.
    ///
    /// Returns `DomainError::EmptyField` if `cli_path` is blank.
    pub fn new(cli_path: String, cli_args: Vec<String>) -> Result<Self, DomainError> {
        if cli_path.trim().is_empty() {
            return Err(DomainError::EmptyField(
                "cli_path".to_string(),
            ));
        }
        Ok(Self { cli_path, cli_args })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_config_defaults() {
        // Valid goal creates successfully
        let config = AgentConfig::new("Review PRD goals".to_string());
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.goal, "Review PRD goals");

        // Empty goal returns error
        let err = AgentConfig::new("".to_string());
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), DomainError::EmptyField("goal".to_string()));

        // Whitespace-only goal returns error
        let err = AgentConfig::new("   ".to_string());
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), DomainError::EmptyField("goal".to_string()));
    }

    #[test]
    fn prompt_template_fields() {
        // Valid fields create successfully
        let template = PromptTemplate::new(
            "full-file".to_string(),
            "Review the document.".to_string(),
        );
        assert!(template.is_ok());
        let template = template.unwrap();
        assert_eq!(template.input_bundling, "full-file");
        assert_eq!(template.instructions, "Review the document.");

        // Empty input_bundling returns error
        let err = PromptTemplate::new("".to_string(), "instructions".to_string());
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err(),
            DomainError::EmptyField("input_bundling".to_string())
        );

        // Empty instructions returns error
        let err = PromptTemplate::new("full-file".to_string(), "".to_string());
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err(),
            DomainError::EmptyField("instructions".to_string())
        );

        // Whitespace-only fields return error
        let err = PromptTemplate::new("  ".to_string(), "  ".to_string());
        assert!(err.is_err());
    }

    #[test]
    fn workspace_agent_config_defaults() {
        // Default config uses "pi" and empty args
        let config = WorkspaceAgentConfig::default_config();
        assert_eq!(config.cli_path, "pi");
        assert!(config.cli_args.is_empty());

        // Custom path and args accepted
        let config = WorkspaceAgentConfig::new(
            "custom-agent".to_string(),
            vec!["--verbose".to_string()],
        );
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.cli_path, "custom-agent");
        assert_eq!(config.cli_args, vec!["--verbose"]);

        // Empty cli_path returns error
        let err = WorkspaceAgentConfig::new("".to_string(), vec![]);
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err(),
            DomainError::EmptyField("cli_path".to_string())
        );
    }

    #[test]
    fn agent_config_serialization() {
        let config = AgentConfig::new("test goal".to_string()).unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.goal, config.goal);
    }

    #[test]
    fn prompt_template_serialization() {
        let template =
            PromptTemplate::new("full-file".to_string(), "do it".to_string()).unwrap();
        let json = serde_json::to_string(&template).unwrap();
        let deserialized: PromptTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, template);
    }

    #[test]
    fn workspace_agent_config_serialization() {
        let config =
            WorkspaceAgentConfig::new("pi".to_string(), vec!["--verbose".to_string()]).unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: WorkspaceAgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }
}
