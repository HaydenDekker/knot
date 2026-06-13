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

// ── AgentProfile Errors ────────────────────────────────────────────────────

/// Errors produced when creating or validating an `AgentProfile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentProfileError {
    /// The profile has no name.
    MissingName,
    /// The provider is empty or whitespace-only.
    EmptyProvider,
    /// The model is empty or whitespace-only.
    EmptyModel,
    /// The system prompt is empty or whitespace-only.
    MissingSystemPrompt,
    /// The profile file has no frontmatter delimiters or no closing delimiter.
    InvalidFormat,
}

impl std::fmt::Display for AgentProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentProfileError::MissingName => write!(f, "agent profile must have a name"),
            AgentProfileError::EmptyProvider => {
                write!(f, "agent profile provider must not be empty")
            }
            AgentProfileError::EmptyModel => {
                write!(f, "agent profile model must not be empty")
            }
            AgentProfileError::MissingSystemPrompt => {
                write!(f, "agent profile system_prompt must not be empty")
            }
            AgentProfileError::InvalidFormat => {
                write!(f, "agent profile file has no valid frontmatter")
            }
        }
    }
}

impl std::error::Error for AgentProfileError {}

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
    ///  "<system_prompt>"]
    /// ```
    ///
    /// If `system_prompt` is provided, it is used for `--system-prompt`
    /// instead of `template.instructions`. This is the primary path for
    /// profile-ref knots where the profile's `system_prompt` should
    /// override the default.
    ///
    /// If `tools` is non-empty, appends `--tools <comma-separated-list>`.
    pub fn build_cli_args(
        &self,
        template: &PromptTemplate,
        system_prompt: Option<&str>,
    ) -> Vec<String> {
        let system_prompt = system_prompt.unwrap_or(&template.instructions);
        let mut args: Vec<String> = vec![
            "-p".to_string(),
            "--model".to_string(),
            self.model.clone(),
            "--system-prompt".to_string(),
            system_prompt.to_string(),
        ];
        if !self.tools.is_empty() {
            args.push("--tools".to_string());
            args.push(self.tools.join(","));
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

/// Shared agent configuration that multiple knots can reference.
///
/// Stored as a `.md` file in `profiles/{name}.md` with YAML frontmatter.
/// Knots reference it by `agent-profile-ref: {name}` and may override
/// individual fields (model, tools) inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentProfile {
    /// Profile name (becomes the filename: `profiles/{name}.md`).
    pub name: String,
    /// The LLM provider identifier (e.g. "openai", "anthropic").
    pub provider: String,
    /// The model name to use (e.g. "gpt-4o").
    pub model: String,
    /// Optional list of tool identifiers to enable.
    #[serde(default)]
    pub tools: Vec<String>,
    /// The system prompt given to the agent.
    #[serde(rename = "system-prompt")]
    pub system_prompt: String,
    /// Optional markdown body content from the profile file (after closing
    /// `---` frontmatter delimiter). Not serialized to YAML frontmatter —
    /// read from disk by the filesystem repository.
    #[serde(skip_deserializing, default)]
    pub body: Option<String>,
}

impl AgentProfile {
    /// Create a new `AgentProfile` with all required fields.
    ///
    /// `tools` defaults to an empty list.
    ///
    /// Returns `AgentProfileError` if any required field is blank.
    pub fn new(
        name: String,
        provider: String,
        model: String,
        system_prompt: String,
    ) -> Result<Self, AgentProfileError> {
        if name.trim().is_empty() {
            return Err(AgentProfileError::MissingName);
        }
        if provider.trim().is_empty() {
            return Err(AgentProfileError::EmptyProvider);
        }
        if model.trim().is_empty() {
            return Err(AgentProfileError::EmptyModel);
        }
        if system_prompt.trim().is_empty() {
            return Err(AgentProfileError::MissingSystemPrompt);
        }
        Ok(Self {
            name,
            provider,
            model,
            tools: Vec::new(),
            system_prompt,
            body: None,
        })
    }

    /// Create a new `AgentProfile` with tools.
    pub fn with_tools(
        name: String,
        provider: String,
        model: String,
        tools: Vec<String>,
        system_prompt: String,
    ) -> Result<Self, AgentProfileError> {
        if name.trim().is_empty() {
            return Err(AgentProfileError::MissingName);
        }
        if provider.trim().is_empty() {
            return Err(AgentProfileError::EmptyProvider);
        }
        if model.trim().is_empty() {
            return Err(AgentProfileError::EmptyModel);
        }
        if system_prompt.trim().is_empty() {
            return Err(AgentProfileError::MissingSystemPrompt);
        }
        Ok(Self {
            name,
            provider,
            model,
            tools,
            system_prompt,
            body: None,
        })
    }

    /// Add markdown body content to an existing profile.
    ///
    /// Used by the filesystem repository when reading profile files that
    /// contain markdown body after the closing frontmatter delimiter.
    pub fn with_body(mut self, body: Option<String>) -> Self {
        self.body = body;
        self
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

        let args = config.build_cli_args(&template, None);
        assert_eq!(
            args,
            vec![
                "-p",
                "--model",
                "gpt-4o",
                "--system-prompt",
                "Review this document.",
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

        let args = config.build_cli_args(&template, None);
        // --tools flag with comma-separated list
        assert!(args.contains(&"--tools".to_string()));
        assert!(args.contains(&"fs,web".to_string()));
    }

    #[test]
    fn agent_config_build_cli_args_with_overridden_system_prompt() {
        let config = AgentConfig::new(
            "goal".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
        )
        .unwrap();
        let template = PromptTemplate::new(
            "full-file".to_string(),
            "Task instructions.".to_string(),
        )
        .unwrap();

        // When system_prompt is provided, it overrides template.instructions
        let custom_prompt = "You are a specialized agent.";
        let args = config.build_cli_args(&template, Some(custom_prompt));
        assert_eq!(
            args,
            vec![
                "-p",
                "--model",
                "gpt-4o",
                "--system-prompt",
                "You are a specialized agent.",
            ]
        );
    }

    #[test]
    fn agent_config_build_cli_args_with_tools_and_overridden_system_prompt() {
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

        let custom_prompt = "Be thorough and detailed.";
        let args = config.build_cli_args(&template, Some(custom_prompt));
        // --system-prompt uses the override
        assert!(args.contains(&"Be thorough and detailed.".to_string()));
        // --tools flag still works
        assert!(args.contains(&"--tools".to_string()));
        assert!(args.contains(&"fs,web".to_string()));
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

    // ── AgentProfile Tests ──────────────────────────────────────────────────

    #[test]
    fn agent_profile_new_valid() {
        let profile = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are a fast reviewer.".to_string(),
        );
        assert!(profile.is_ok());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "fast");
        assert_eq!(profile.provider, "openai");
        assert_eq!(profile.model, "gpt-4o");
        assert!(profile.tools.is_empty());
        assert_eq!(profile.system_prompt, "You are a fast reviewer.");
    }

    #[test]
    fn agent_profile_with_tools() {
        let profile = AgentProfile::with_tools(
            "full-stack".to_string(),
            "anthropic".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            vec!["fs".to_string(), "web".to_string()],
            "You are a full-stack reviewer.".to_string(),
        );
        assert!(profile.is_ok());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "full-stack");
        assert_eq!(profile.tools, vec!["fs", "web"]);
    }

    #[test]
    fn agent_profile_missing_name() {
        let result = AgentProfile::new(
            "".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "system prompt".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::MissingName);
    }

    #[test]
    fn agent_profile_whitespace_name() {
        let result = AgentProfile::new(
            "   ".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "system prompt".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::MissingName);
    }

    #[test]
    fn agent_profile_empty_provider() {
        let result = AgentProfile::new(
            "fast".to_string(),
            "".to_string(),
            "gpt-4o".to_string(),
            "system prompt".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::EmptyProvider);
    }

    #[test]
    fn agent_profile_empty_model() {
        let result = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "".to_string(),
            "system prompt".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AgentProfileError::EmptyModel);
    }

    #[test]
    fn agent_profile_empty_system_prompt() {
        let result = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AgentProfileError::MissingSystemPrompt
        );
    }

    #[test]
    fn agent_profile_whitespace_system_prompt() {
        let result = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "   ".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AgentProfileError::MissingSystemPrompt
        );
    }

    #[test]
    fn agent_profile_serialization() {
        let profile = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap();
        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, profile);
    }

    #[test]
    fn agent_profile_serialization_with_tools() {
        let profile = AgentProfile::with_tools(
            "full".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            vec!["fs".to_string(), "web".to_string()],
            "You are full.".to_string(),
        )
        .unwrap();
        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, profile);
        assert_eq!(deserialized.tools, vec!["fs", "web"]);
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
            AgentProfileError::MissingSystemPrompt.to_string(),
            "agent profile system_prompt must not be empty"
        );
    }

    #[test]
    fn agent_profile_with_multiline_system_prompt() {
        let profile = AgentProfile::new(
            "detailed".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are a detailed reviewer.\n\nKeep responses thorough.".to_string(),
        );
        assert!(profile.is_ok());
        let profile = profile.unwrap();
        assert!(profile.system_prompt.contains("\n\n"));
    }
}
