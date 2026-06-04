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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentConfig {
    /// The goal this knot's agent should accomplish.
    pub goal: String,
    /// The LLM provider identifier (e.g. "openai", "anthropic").
    pub provider: String,
    /// The model name to use (e.g. "gpt-4o").
    pub model: String,
    /// Optional list of tool identifiers to enable.
    #[serde(default)]
    pub tools: Vec<String>,
}

impl AgentConfig {
    /// Create a new `AgentConfig` with goal, provider, and model.
    ///
    /// `tools` defaults to an empty list.
    ///
    /// Returns `DomainError::EmptyField` if any required field is blank.
    pub fn new(
        goal: String,
        provider: String,
        model: String,
    ) -> Result<Self, DomainError> {
        if goal.trim().is_empty() {
            return Err(DomainError::EmptyField("goal".to_string()));
        }
        if provider.trim().is_empty() {
            return Err(DomainError::EmptyField("provider".to_string()));
        }
        if model.trim().is_empty() {
            return Err(DomainError::EmptyField("model".to_string()));
        }
        Ok(Self {
            goal,
            provider,
            model,
            tools: Vec::new(),
        })
    }

    /// Build a list of `pi` CLI arguments from this config and a
    /// `PromptTemplate`.
    ///
    /// Produces arguments in the format:
    /// ```text
    /// ["-p", "--model", "<model>", "--system-prompt",
    ///  "<instructions>", "--no-session", "--no-tools"]
    /// ```
    ///
    /// If `tools` is non-empty, `--no-tools` is omitted and each tool is
    /// added as `--tool <name>`.
    pub fn build_cli_args(&self, template: &PromptTemplate) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        args.push("-p".to_string());
        args.push("--model".to_string());
        args.push(self.model.clone());
        args.push("--system-prompt".to_string());
        args.push(template.instructions.clone());
        args.push("--no-session".to_string());
        if self.tools.is_empty() {
            args.push("--no-tools".to_string());
        } else {
            for tool in &self.tools {
                args.push("--tool".to_string());
                args.push(tool.clone());
            }
        }
        args
    }
}

/// Prompt template used when executing a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
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

/// Rig-level agent configuration. One config per rig,
/// shared by all knots in that rig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RigAgentConfig {
    /// Path to the agent CLI binary.
    pub cli_path: String,
    /// Arguments passed to the CLI.
    pub cli_args: Vec<String>,
}

impl RigAgentConfig {
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
        // Valid fields create successfully
        let config = AgentConfig::new(
            "Review PRD goals".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
        );
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.goal, "Review PRD goals");
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4o");
        assert!(config.tools.is_empty());

        // Empty goal returns error
        let err = AgentConfig::new("".to_string(), "openai".to_string(), "gpt-4o".to_string());
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), DomainError::EmptyField("goal".to_string()));

        // Whitespace-only goal returns error
        let err = AgentConfig::new("   ".to_string(), "openai".to_string(), "gpt-4o".to_string());
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), DomainError::EmptyField("goal".to_string()));

        // Empty provider returns error
        let err = AgentConfig::new("goal".to_string(), "".to_string(), "gpt-4o".to_string());
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), DomainError::EmptyField("provider".to_string()));

        // Empty model returns error
        let err = AgentConfig::new("goal".to_string(), "openai".to_string(), "".to_string());
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), DomainError::EmptyField("model".to_string()));
    }

    #[test]
    fn agent_config_with_provider_and_model() {
        let config = AgentConfig::new(
            "review".to_string(),
            "anthropic".to_string(),
            "claude-sonnet-4-20250514".to_string(),
        )
        .unwrap();

        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet-4-20250514");
        assert!(config.tools.is_empty());
    }

    #[test]
    fn agent_config_build_cli_args_basic() {
        let config = AgentConfig::new(
            "goal".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
        )
        .unwrap();
        let template = PromptTemplate::new(
            "full-file".to_string(),
            "Review this document.".to_string(),
        )
        .unwrap();

        let args = config.build_cli_args(&template);
        assert_eq!(
            args,
            vec![
                "-p",
                "--model",
                "gpt-4o",
                "--system-prompt",
                "Review this document.",
                "--no-session",
                "--no-tools",
            ]
        );
    }

    #[test]
    fn agent_config_build_cli_args_with_tools() {
        let mut config = AgentConfig::new(
            "goal".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
        )
        .unwrap();
        config.tools = vec!["fs".to_string(), "web".to_string()];
        let template = PromptTemplate::new(
            "full-file".to_string(),
            "Do something.".to_string(),
        )
        .unwrap();

        let args = config.build_cli_args(&template);
        // --no-tools should NOT appear; individual --tool flags instead
        assert!(!args.contains(&"--no-tools".to_string()));
        assert!(args.contains(&"--tool".to_string()));
        assert!(args.contains(&"fs".to_string()));
        assert!(args.contains(&"web".to_string()));
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
    fn rig_agent_config_defaults() {
        // Default config uses "pi" and empty args
        let config = RigAgentConfig::default_config();
        assert_eq!(config.cli_path, "pi");
        assert!(config.cli_args.is_empty());

        // Custom path and args accepted
        let config = RigAgentConfig::new(
            "custom-agent".to_string(),
            vec!["--verbose".to_string()],
        );
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.cli_path, "custom-agent");
        assert_eq!(config.cli_args, vec!["--verbose"]);

        // Empty cli_path returns error
        let err = RigAgentConfig::new("".to_string(), vec![]);
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err(),
            DomainError::EmptyField("cli_path".to_string())
        );
    }

    #[test]
    fn agent_config_serialization() {
        let config = AgentConfig::new(
            "test goal".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
        )
        .unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
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
    fn rig_agent_config_serialization() {
        let config =
            RigAgentConfig::new("pi".to_string(), vec!["--verbose".to_string()]).unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: RigAgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }
}
