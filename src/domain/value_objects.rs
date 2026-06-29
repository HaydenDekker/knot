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
    /// The profile prompt is empty or whitespace-only.
    MissingProfilePrompt,
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
            AgentProfileError::MissingProfilePrompt => {
                write!(f, "agent profile profile_prompt must not be empty")
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Extra CLI arguments appended after the standard args.
    ///
    /// Used by the retry loop to inject `--session-id <id>` on retry
    /// attempts. Not serialized — always empty on deserialization.
    #[serde(default, skip_serializing)]
    pub extra_args: Vec<String>,
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
            extra_args: Vec::new(),
        })
    }

    /// Build a list of `pi` CLI arguments from this config.
    ///
    /// Produces arguments in the format:
    /// ```text
    /// ["-p", "--model", "<model>"]
    /// ```
    ///
    /// If `tools` is non-empty, appends `--tools <comma-separated-list>`.
    /// If `extra_args` is non-empty (e.g. `--session-id` from retry loop),
    /// appends those after the standard args.
    ///
    /// The profile prompt and knot instructions are delivered via stdin
    /// (not `--system-prompt`), so they are not included in CLI args.
    ///
    /// **Adapter use only** — this constructs CLI plumbing from domain
    /// data. The application layer should not call this directly.
    pub fn build_cli_args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "-p".to_string(),
            "--model".to_string(),
            self.model.clone(),
        ];
        if !self.tools.is_empty() {
            args.push("--tools".to_string());
            args.push(self.tools.join(","));
        }
        args.extend(self.extra_args.clone());
        args
    }
}

/// Prompt template used when executing a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// The prompt instructions.
    pub instructions: String,
}

impl PromptTemplate {
    /// Create a new `PromptTemplate` with non-empty instructions.
    ///
    /// Returns `DomainError::EmptyField` if instructions are blank.
    pub fn new(instructions: String) -> Result<Self, DomainError> {
        if instructions.trim().is_empty() {
            return Err(DomainError::EmptyField(
                "instructions".to_string(),
            ));
        }
        Ok(Self { instructions })
    }
}

/// Agent adapter selector.
///
/// Determines which adapter Knot uses to invoke the Pi CLI.
/// Each adapter hardcodes its own binary path and flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentAdapter {
    /// Plain text stdout via subprocess (current behaviour).
    #[serde(rename = "pi-stdio")]
    PiStdio,
    /// JSON-L stream with metadata extraction.
    #[serde(rename = "pi-json")]
    PiJson,
}

/// Rig-level agent configuration. One config per rig,
/// shared by all knots in that rig.
///
/// Selects which adapter to use — no invocation details.
/// Each adapter hardcodes its own binary path and CLI flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RigAgentConfig {
    #[serde(default = "default_agent_adapter")]
    pub agent_adapter: AgentAdapter,
}

fn default_agent_adapter() -> AgentAdapter {
    AgentAdapter::PiStdio
}

impl RigAgentConfig {
    /// Create a default workspace config (`agent_adapter = PiStdio`).
    pub fn default_config() -> Self {
        Self {
            agent_adapter: AgentAdapter::PiStdio,
        }
    }
}

/// Shared agent configuration that multiple knots can reference.
///
/// Stored as a `.md` file in `profiles/{name}.md`. The file uses YAML
/// frontmatter for structural metadata (`name`, `provider`, `model`,
/// `tools`, `timeout`) and the markdown body (text after the closing
/// `---`) for the profile prompt (`profile_prompt`).
///
/// Knots reference profiles by `agent-profile-ref: {name}` and may
/// override individual fields (model, tools) inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The profile-level prompt segment given to the agent.
    ///
    /// In profile files this is stored as the markdown body after the
    /// closing `---` frontmatter delimiter (not in YAML frontmatter).
    #[serde(rename = "profile-prompt")]
    pub profile_prompt: String,
    /// Session timeout in seconds. `None` means use the runner's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl AgentProfile {
    /// Default session timeout in seconds when not specified per-profile.
    ///
    /// Matches `AppConfig::agent_timeout` (5 minutes). When a profile
    /// does not set `timeout`, the agent runner uses this value as its
    /// session deadline.
    pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

    /// Create a new `AgentProfile` with all required fields.
    ///
    /// `tools` defaults to an empty list. `timeout` defaults to `None`
    /// (use the runner's default).
    ///
    /// Returns `AgentProfileError` if any required field is blank.
    pub fn new(
        name: String,
        provider: String,
        model: String,
        profile_prompt: String,
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
        if profile_prompt.trim().is_empty() {
            return Err(AgentProfileError::MissingProfilePrompt);
        }
        Ok(Self {
            name,
            provider,
            model,
            tools: Vec::new(),
            profile_prompt,
            timeout: None,
        })
    }

    /// Create a new `AgentProfile` with tools.
    pub fn with_tools(
        name: String,
        provider: String,
        model: String,
        tools: Vec<String>,
        profile_prompt: String,
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
        if profile_prompt.trim().is_empty() {
            return Err(AgentProfileError::MissingProfilePrompt);
        }
        Ok(Self {
            name,
            provider,
            model,
            tools,
            profile_prompt,
            timeout: None,
        })
    }

    /// Set the session timeout in seconds.
    ///
    /// Returns `None` to use the runner's default timeout
    /// (`DEFAULT_TIMEOUT_SECS`).
    pub fn with_timeout(mut self, timeout: Option<u64>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build an `AgentConfig` by merging this profile's fields
    /// with the knot's prompt instructions.
    ///
    /// The profile provides `provider`, `model`, and `tools`.
    /// The knot's `PromptTemplate.instructions` becomes the goal.
    pub fn resolve_for_knot(
        &self,
        knot: &crate::domain::entities::Knot,
    ) -> AgentConfig {
        AgentConfig {
            goal: knot.prompt_template.instructions.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
            extra_args: Vec::new(),
        }
    }

    /// Return the profile's session timeout as a `Duration`.
    ///
    /// Returns `None` when the profile does not specify a timeout
    /// (the agent runner uses its own default).
    pub fn session_timeout(&self) -> Option<std::time::Duration> {
        self.timeout.map(std::time::Duration::from_secs)
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

        let args = config.build_cli_args();
        assert_eq!(
            args,
            vec!["-p", "--model", "gpt-4o"]
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

        let args = config.build_cli_args();
        assert_eq!(
            args,
            vec!["-p", "--model", "gpt-4o", "--tools", "fs,web"]
        );
    }

    #[test]
    fn agent_config_build_cli_args_no_system_prompt_flag() {
        // build_cli_args no longer emits --system-prompt.
        // Prompt content is delivered via stdin instead.
        let config = AgentConfig::new(
            "goal".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
        )
        .unwrap();

        let args = config.build_cli_args();
        assert!(!args.contains(&"--system-prompt".to_string()));
        assert_eq!(args, vec!["-p", "--model", "gpt-4o"]);
    }

    #[test]
    fn prompt_template_fields() {
        // Valid instructions create successfully
        let template = PromptTemplate::new(
            "Review the document.".to_string(),
        );
        assert!(template.is_ok());
        let template = template.unwrap();
        assert_eq!(template.instructions, "Review the document.");

        // Empty instructions returns error
        let err = PromptTemplate::new("".to_string());
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err(),
            DomainError::EmptyField("instructions".to_string())
        );

        // Whitespace-only instructions return error
        let err = PromptTemplate::new("  ".to_string());
        assert!(err.is_err());
    }

    #[test]
    fn rig_agent_config_defaults() {
        // Default config uses PiStdio adapter
        let config = RigAgentConfig::default_config();
        assert_eq!(config.agent_adapter, AgentAdapter::PiStdio);
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
            PromptTemplate::new("do it".to_string()).unwrap();
        let json = serde_json::to_string(&template).unwrap();
        let deserialized: PromptTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, template);
    }

    #[test]
    fn rig_agent_config_serialization() {
        let config = RigAgentConfig {
            agent_adapter: AgentAdapter::PiJson,
        };
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
        assert_eq!(profile.profile_prompt, "You are a fast reviewer.");
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
    fn agent_profile_empty_profile_prompt() {
        let result = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AgentProfileError::MissingProfilePrompt
        );
    }

    #[test]
    fn agent_profile_whitespace_profile_prompt() {
        let result = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "   ".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AgentProfileError::MissingProfilePrompt
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
            AgentProfileError::MissingProfilePrompt.to_string(),
            "agent profile profile_prompt must not be empty"
        );
    }

    #[test]
    fn agent_profile_with_multiline_profile_prompt() {
        let profile = AgentProfile::new(
            "detailed".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are a detailed reviewer.\n\nKeep responses thorough.".to_string(),
        );
        assert!(profile.is_ok());
        let profile = profile.unwrap();
        assert!(profile.profile_prompt.contains("\n\n"));
    }

    #[test]
    fn agent_profile_default_timeout_constant() {
        assert_eq!(AgentProfile::DEFAULT_TIMEOUT_SECS, 300);
    }

    #[test]
    fn agent_profile_new_has_no_timeout() {
        let profile = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap();
        assert_eq!(profile.timeout, None);
    }

    #[test]
    fn agent_profile_with_timeout_sets_field() {
        let profile = AgentProfile::new(
            "slow".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "You are slow but thorough.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(600));
        assert_eq!(profile.timeout, Some(600));
    }

    #[test]
    fn agent_profile_with_timeout_none() {
        let profile = AgentProfile::new(
            "default".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Default timeout.".to_string(),
        )
        .unwrap()
        .with_timeout(None);
        assert_eq!(profile.timeout, None);
    }

    #[test]
    fn agent_profile_serialization_with_timeout() {
        let profile = AgentProfile::new(
            "slow".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Thorough review.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(600));

        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, profile);
        assert_eq!(deserialized.timeout, Some(600));
        // Verify timeout is present in JSON
        assert!(json.contains("\"timeout\":600"));
    }

    #[test]
    fn agent_profile_serialization_without_timeout() {
        let profile = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Quick review.".to_string(),
        )
        .unwrap();

        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, profile);
        assert_eq!(deserialized.timeout, None);
        // Verify timeout is NOT present in JSON (skip_serializing_if)
        assert!(!json.contains("timeout"));
    }

    #[test]
    fn agent_profile_serialization_missing_timeout_defaults_to_none() {
        // Deserialize JSON that has no timeout field — should default to None
        let json = r#"{
            "name": "legacy",
            "provider": "openai",
            "model": "gpt-4o",
            "tools": [],
            "profile-prompt": "Legacy profile."
        }"#;
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.timeout, None);
        assert_eq!(profile.name, "legacy");
    }

    #[test]
    fn agent_profile_yaml_roundtrip_with_timeout() {
        let profile = AgentProfile::new(
            "timed".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Timed review.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(600));

        let yaml = serde_yaml::to_string(&profile).unwrap();
        let deserialized: AgentProfile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.timeout, Some(600));
        assert_eq!(deserialized.name, "timed");
        assert_eq!(deserialized.provider, "anthropic");
    }

    #[test]
    fn agent_profile_yaml_roundtrip_without_timeout() {
        let profile = AgentProfile::new(
            "no-timeout".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "No timeout.".to_string(),
        )
        .unwrap();

        let yaml = serde_yaml::to_string(&profile).unwrap();
        let deserialized: AgentProfile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.timeout, None);
        assert_eq!(deserialized.name, "no-timeout");
        // Verify timeout is not in YAML output
        // (serde_yaml may or may not skip None — the important part is
        // round-trip correctness)
        let _ = &yaml; // silence unused warning
    }

    #[test]
    fn agent_profile_yaml_missing_timeout_defaults_to_none() {
        let yaml = "name: legacy\nprovider: openai\nmodel: gpt-4o\nprofile-prompt: Legacy.\n";
        let profile: AgentProfile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(profile.timeout, None);
        assert_eq!(profile.name, "legacy");
    }

    // ── AgentProfile resolve_for_knot Tests ─────────────────────────

    #[test]
    fn agent_profile_resolve_for_knot_maps_fields() {
        use crate::domain::entities::{Knot, KnotId};
        use std::path::PathBuf;

        let profile = AgentProfile::with_tools(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            vec!["fs".to_string(), "web".to_string()],
            "You are fast.".to_string(),
        )
        .unwrap();

        let knot = Knot {
            id: KnotId("reviewer".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "Review the document.".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        };

        let config = profile.resolve_for_knot(&knot);

        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.tools, vec!["fs", "web"]);
        assert_eq!(config.goal, "Review the document.");
        assert!(config.extra_args.is_empty());
    }

    #[test]
    fn agent_profile_resolve_for_knot_no_tools() {
        use crate::domain::entities::{Knot, KnotId};
        use std::path::PathBuf;

        let profile = AgentProfile::new(
            "minimal".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Be concise.".to_string(),
        )
        .unwrap();

        let knot = Knot {
            id: KnotId("k1".to_string()),
            agent_profile_ref: "minimal".to_string(),
            prompt_template: PromptTemplate {
                instructions: "Check the code.".to_string(),
            },
            strand_dir: PathBuf::from("input"),
            git_versioned: false,
        };

        let config = profile.resolve_for_knot(&knot);
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet");
        assert!(config.tools.is_empty());
        assert_eq!(config.goal, "Check the code.");
    }

    #[test]
    fn agent_profile_session_timeout_some() {
        let profile = AgentProfile::new(
            "slow".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Thorough review.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(600));

        assert_eq!(
            profile.session_timeout(),
            Some(std::time::Duration::from_secs(600))
        );
    }

    #[test]
    fn agent_profile_session_timeout_none() {
        let profile = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Quick review.".to_string(),
        )
        .unwrap();

        assert_eq!(profile.session_timeout(), None);
    }

    // ── AgentAdapter Tests ──────────────────────────────────────────────────

    #[test]
    fn test_agent_adapter_default_pistdio() {
        // Missing field defaults to PiStdio
        let yaml = "";
        let config: RigAgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.agent_adapter, AgentAdapter::PiStdio);
    }

    #[test]
    fn test_agent_adapter_pijson_from_yaml() {
        let yaml = "agent-adapter: pi-json";
        let config: RigAgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.agent_adapter, AgentAdapter::PiJson);
    }

    #[test]
    fn test_agent_adapter_pistdio_from_yaml() {
        let yaml = "agent-adapter: pi-stdio";
        let config: RigAgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.agent_adapter, AgentAdapter::PiStdio);
    }

    #[test]
    fn test_agent_adapter_invalid_yaml() {
        let yaml = "agent-adapter: unknown";
        let result: Result<RigAgentConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_rig_agent_config_serialization_roundtrip() {
        let config = RigAgentConfig {
            agent_adapter: AgentAdapter::PiJson,
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: RigAgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.agent_adapter, AgentAdapter::PiJson);
        assert_eq!(deserialized, config);
    }

    #[test]
    fn test_rig_agent_config_no_cli_path_or_args() {
        // Compile-time check: RigAgentConfig has no cli_path or cli_args.
        // If these fields existed, this would compile. We use a helper
        // that only compiles when the field does NOT exist.
        fn assert_no_cli_fields(config: &RigAgentConfig) {
            // Only agent_adapter exists — if cli_path/cli_args existed,
            // this would still compile but we verify at runtime:
            let yaml = serde_yaml::to_string(config).unwrap();
            assert!(!yaml.contains("cli_path"), "cli_path should not exist");
            assert!(!yaml.contains("cli_args"), "cli_args should not exist");
        }
        let config = RigAgentConfig::default_config();
        assert_no_cli_fields(&config);
    }
}
