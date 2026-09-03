//! JSON agent runner — invokes the Pi CLI with `--mode json` and parses
//! the JSON-L output stream.
//!
//! Reads stdout line-by-line as newline-delimited JSON, extracting:
//! - Session ID from the first `session` event
//! - Token usage from `agent_end` usage data
//! - Response text from `agent_end` message content (filtered by `stopReason`)
//!
//! Falls back to raw stdout if JSON-L parsing fails.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::adapters::live_output::{
    join_all, spawn_reader, spawn_watchdog, KillReason, LiveOutput,
};
use crate::application::ports::{
    AgentInvocationMetadata, AgentOutput, AgentRunner, CompactionRecord,
    ExecutionContext, PortError, TokenUsage,
};
use crate::domain::entities::StrandPath;
use crate::domain::value_objects::AgentConfig;

/// JSON-L implementation of [`AgentRunner`].
///
/// Appends `--mode json` to CLI arguments, spawns the child process,
/// and parses stdout as newline-delimited JSON events.
#[derive(Debug, Clone)]
pub struct PiJsonAgentRunner {
    /// Maximum duration the agent may run before being killed.
    /// Defaults to 120 seconds.
    timeout: Duration,
    /// Plan 081: kill the session when it produces no output for this
    /// window. `None` disables the inactivity watchdog (the total
    /// timeout still bounds the attempt).
    inactivity_timeout: Option<Duration>,
    /// Path to the agent CLI binary. Resolved once at construction time
    /// to avoid PATH lookup races at execution time.
    cli_path: String,
}

impl Default for PiJsonAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            inactivity_timeout: None,
            cli_path: Self::resolve_cli_path(),
        }
    }
}

impl PiJsonAgentRunner {
    /// Create a new runner with the default 120-second timeout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new runner with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            inactivity_timeout: None,
            cli_path: Self::resolve_cli_path(),
        }
    }

    /// Create a new runner with a custom total timeout and inactivity
    /// window (plan 081).
    ///
    /// Used by the composition root (`build_app_context`) when
    /// `AppConfig::cli_path` is not set: the CLI path is resolved via
    /// [`Self::resolve_cli_path`]; the inactivity window kills a silent
    /// session early (`None` disables the watchdog).
    pub fn with_timeouts(
        timeout: Duration,
        inactivity_timeout: Option<Duration>,
    ) -> Self {
        Self {
            timeout,
            inactivity_timeout,
            cli_path: Self::resolve_cli_path(),
        }
    }

    /// Create a new runner with a specific CLI path.
    /// Used by integration tests to inject a mock binary.
    #[cfg(test)]
    pub fn with_cli_path(cli_path: String) -> Self {
        Self {
            timeout: Duration::from_secs(120),
            inactivity_timeout: None,
            cli_path,
        }
    }

    /// Create a new runner with an explicit CLI path and timeout.
    ///
    /// Used by the composition root (`build_app_context`) when
    /// `AppConfig::cli_path` is set.
    pub fn with_cli_path_and_timeout(cli_path: String, timeout: Duration) -> Self {
        Self {
            timeout,
            inactivity_timeout: None,
            cli_path,
        }
    }

    /// Create a new runner with an explicit CLI path, total timeout,
    /// and inactivity window (plan 081).
    ///
    /// Used by the composition root (`build_app_context`): the total
    /// timeout bounds the whole attempt (existing behaviour); the
    /// inactivity window kills a silent session early (`None` disables
    /// the watchdog).
    pub fn with_cli_path_and_timeouts(
        cli_path: String,
        timeout: Duration,
        inactivity_timeout: Option<Duration>,
    ) -> Self {
        Self {
            timeout,
            inactivity_timeout,
            cli_path,
        }
    }

    /// Resolve the CLI path for the agent binary.
    ///
    /// Checks the `KNOT_TEST_CLI_PATH` environment variable first (set by
    /// integration test helpers), then falls back to PATH lookup of `"pi"`.
    fn resolve_cli_path() -> String {
        std::env::var("KNOT_TEST_CLI_PATH").unwrap_or_else(|_| "pi".to_string())
    }

    /// Append `--mode json` flags to CLI args.
    fn build_json_cli_args(cli_args: &[String]) -> Vec<String> {
        let mut args = cli_args.to_vec();
        args.push("--mode".to_string());
        args.push("json".to_string());
        args
    }

    /// Build the prompt with profile prompt, trigger line, and knot
    /// instructions.
    fn build_prompt_with_context(
        ctx: &ExecutionContext,
        profile_prompt: &str,
    ) -> String {
        let mut full_prompt = String::new();

        if !profile_prompt.is_empty() {
            full_prompt.push_str(profile_prompt);
            full_prompt.push_str("\n\n");
        }

        full_prompt.push_str(&ctx.prompt);

        if !ctx.event_type.is_empty() {
            full_prompt.push_str("\n\n");
            full_prompt.push_str(&format!(
                "**{}** triggered by **{}** on **{}**",
                ctx.knot_name.as_deref().unwrap_or("unknown"),
                ctx.event_type,
                ctx.strand_path.0.display()
            ));
        }

        full_prompt
    }

    /// Parse a single JSON-L line and update tracked state.
    ///
    /// Returns `true` if the line was valid JSON, `false` otherwise.
    /// When `false` is returned the caller should treat output as raw.
    fn parse_json_line(
        line: &str,
        session_id: &mut Option<String>,
        response_text: &mut String,
        token_usage: &mut Option<TokenUsage>,
        compactions: &mut Vec<CompactionRecord>,
        error_message: &mut Option<String>,
    ) -> bool {
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let event_type = value.get("type").and_then(|t| t.as_str());

        match event_type {
            Some("session") => {
                if let Some(id) = value.get("id").and_then(|id| id.as_str()) {
                    *session_id = Some(id.to_string());
                }
            }
            Some("message_end") => {
                // Ignored — agent_end has the complete message array with
                // stopReason filtering. message_end cannot distinguish
                // intermediate tool-use messages from final responses.
            }
            Some("compaction_end") => {
                // Plan 079: record the compaction for loom-log visibility
                // and terminal-overflow detection. `result` is absent on
                // failed compactions (pi omits undefined keys in JSON),
                // so `tokens_before` is `None` there.
                compactions.push(CompactionRecord {
                    reason: value
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    tokens_before: value
                        .get("result")
                        .and_then(|r| r.get("tokensBefore"))
                        .and_then(|t| t.as_u64()),
                    will_retry: value
                        .get("willRetry")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false),
                    error: value
                        .get("errorMessage")
                        .and_then(|e| e.as_str())
                        .map(String::from),
                });
            }
            Some("agent_end") => {
                // Extract token usage from usage object
                if let Some(usage_obj) = value.get("usage") {
                    *token_usage = Some(TokenUsage {
                        input: usage_obj
                            .get("input")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        output: usage_obj
                            .get("output")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        cache_read: usage_obj
                            .get("cache_read")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        cache_write: usage_obj
                            .get("cache_write")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        total: usage_obj
                            .get("total")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                    });
                }

                // Extract response text from messages array
                // Format: messages[].content[].text
                if let Some(messages) = value.get("messages") {
                    if let Some(arr) = messages.as_array() {
                        for msg in arr {
                            if let Some(role) =
                                msg.get("role").and_then(|r| r.as_str())
                            {
                                if role == "assistant" {
                                    let stop_reason = msg.get("stopReason")
                                        .and_then(|r| r.as_str());
                                    // Plan 080: capture the provider error
                                    // message of failed turns for overflow
                                    // classification (see
                                    // `is_context_overflow_message`). When
                                    // pi's compaction never ran, this is
                                    // the only overflow signal in the stream.
                                    if stop_reason == Some("error") {
                                        if let Some(err) = msg
                                            .get("errorMessage")
                                            .and_then(|e| e.as_str())
                                        {
                                            *error_message =
                                                Some(err.to_string());
                                        }
                                    }
                                    // Only include final responses, not intermediate
                                    // tool-use messages (stopReason: "toolUse").
                                    let is_final = matches!(
                                        stop_reason,
                                        Some("stop") | Some("length")
                                    );
                                    if is_final {
                                        if let Some(content) = msg.get("content") {
                                            if let Some(carr) = content.as_array() {
                                                for item in carr {
                                                    if let Some(text) =
                                                        item.get("text").and_then(|t| t.as_str())
                                                    {
                                                        response_text.push_str(text);
                                                    }
                                                }
                                            } else if let Some(text) =
                                                content.as_str()
                                            {
                                                response_text.push_str(text);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        true
    }

    /// Parse JSON-L from raw stdout, extracting session_id, response,
    /// token usage, compaction events, and the failed turn's provider
    /// error message. Returns `true` if all lines parsed as valid JSON.
    fn parse_stdout(
        raw_stdout: &str,
    ) -> (
        bool,
        Option<String>,
        String,
        Option<TokenUsage>,
        Vec<CompactionRecord>,
        Option<String>,
    ) {
        let mut session_id: Option<String> = None;
        let mut response_text = String::new();
        let mut token_usage: Option<TokenUsage> = None;
        let mut compactions: Vec<CompactionRecord> = Vec::new();
        let mut error_message: Option<String> = None;
        let mut had_parse_error = false;

        for line in raw_stdout.lines() {
            if line.is_empty() {
                continue;
            }
            if !Self::parse_json_line(
                line,
                &mut session_id,
                &mut response_text,
                &mut token_usage,
                &mut compactions,
                &mut error_message,
            ) {
                had_parse_error = true;
            }
        }

        (
            had_parse_error,
            session_id,
            response_text,
            token_usage,
            compactions,
            error_message,
        )
    }

    /// Plan 081: name the blocked call from the accumulated JSON-L
    /// stream — the most recent `tool_execution_start` with no
    /// matching `tool_execution_end`, as
    /// `{toolName}({args preview})`. The args preview is the `command`
    /// field when present, else the first 80 chars of the args JSON.
    ///
    /// Best-effort: malformed lines are ignored and `None` is fine —
    /// the restart note works without the call name.
    fn parse_blocked_call(raw_stdout: &str) -> Option<String> {
        #[derive(Debug)]
        struct OpenCall {
            id: String,
            label: String,
        }
        let mut open: Vec<OpenCall> = Vec::new();
        for line in raw_stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // malformed lines ignored
            };
            match value.get("type").and_then(|t| t.as_str()) {
                Some("tool_execution_start") => {
                    let id = value
                        .get("toolCallId")
                        .and_then(|i| i.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let tool_name = value
                        .get("toolName")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    let args = value.get("args");
                    let args_preview = if let Some(cmd) = args
                        .and_then(|a| a.get("command"))
                        .and_then(|c| c.as_str())
                    {
                        cmd.to_string()
                    } else if let Some(a) = args {
                        let json = a.to_string();
                        let mut preview: String = json.chars().take(80).collect();
                        if json.chars().count() > 80 {
                            preview.push('…');
                        }
                        preview
                    } else {
                        String::new()
                    };
                    open.push(OpenCall {
                        id,
                        label: format!("{tool_name}({args_preview})"),
                    });
                }
                Some("tool_execution_end") => {
                    if let Some(id) = value.get("toolCallId").and_then(|i| i.as_str()) {
                        if let Some(pos) = open.iter().position(|c| c.id == id) {
                            open.remove(pos);
                        }
                    }
                }
                _ => {}
            }
        }
        open.last().map(|c| c.label.clone())
    }

    /// The failing record when the stream shows a terminal overflow:
    /// an overflow compaction with `will_retry: false` that follows an
    /// overflow compaction with `will_retry: true` (recovery ran, the
    /// context still does not fit).
    fn terminal_overflow(
        records: &[CompactionRecord],
    ) -> Option<&CompactionRecord> {
        let failing_idx = records
            .iter()
            .rposition(|r| r.reason == "overflow" && !r.will_retry)?;
        if records[..failing_idx]
            .iter()
            .any(|r| r.reason == "overflow" && r.will_retry)
        {
            Some(&records[failing_idx])
        } else {
            // A `willRetry: false` overflow without a preceding
            // successful compaction means compaction could not even run
            // (missing model/auth, transient summarisation API error) —
            // not terminal: the nudge loop's fresh user message gives
            // pi a new recovery attempt.
            None
        }
    }

    /// Classify a provider error message as a context overflow.
    ///
    /// Plan 080: when pi's compaction never ran (disabled in pi
    /// settings — e.g. a rig initialised before knot-init seeded
    /// `.pi/settings.json`), an over-full context surfaces only as the
    /// provider's error message on the `stopReason: "error"` message —
    /// no `compaction_end` events, so [`Self::terminal_overflow`] finds
    /// nothing and the failure degenerates into plan 078's nudge loop,
    /// which re-enters the same over-full session and burns every
    /// retry. Nudging cannot succeed (each nudge adds tokens), so the
    /// signature is matched here and the strand fails fast with
    /// [`PortError::ContextLimitReached`].
    ///
    /// Substrings mirror the common cases of pi's `OVERFLOW_PATTERNS`
    /// (pi-ai `utils/overflow.js`), kept as plain case-insensitive
    /// substrings. Bedrock-style throttling ("Too many tokens, please
    /// wait…") is excluded, as in pi's `NON_OVERFLOW_PATTERNS`.
    pub fn is_context_overflow_message(message: &str) -> bool {
        const NON_OVERFLOW: &[&str] = &[
            "rate limit",
            "too many requests",
            "throttl",
        ];
        const SIGNATURES: &[&str] = &[
            "exceeds the available context size", // llama.cpp server
            "prompt is too long", // Anthropic / Ollama
            "request_too_large", // Anthropic HTTP 413
            "input is too long for requested model", // Amazon Bedrock
            "exceeds the context window", // OpenAI
            "maximum context length", // OpenAI-compatible / OpenRouter / Mistral
            "exceeds the maximum number of tokens", // Google Gemini
            "maximum prompt length is", // xAI
            "reduce the length of the messages", // Groq
            "maximum allowed input length", // OpenRouter / Poolside
            "is longer than the model", // Together AI
            "exceeds the limit of", // GitHub Copilot
            "greater than the context length", // LM Studio
            "context window exceeds limit", // MiniMax
            "exceeded model token limit", // Kimi For Coding
            "model_context_window_exceeded", // z.ai
            "context_length_exceeded", // generic
            "context length exceeded", // generic
            "too many tokens", // generic
            "token limit exceeded", // generic
        ];
        let lower = message.to_lowercase();
        if NON_OVERFLOW.iter().any(|s| lower.contains(s)) {
            return false;
        }
        SIGNATURES.iter().any(|s| lower.contains(s))
    }

    /// Read `compaction.enabled` from a pi settings file.
    ///
    /// Returns `Some(true|false)` when the file exists, parses as
    /// JSON, and carries an explicit boolean `compaction.enabled`;
    /// `None` otherwise (absent file, unparseable JSON, key missing).
    pub fn read_pi_compaction_enabled(
        settings_path: &std::path::Path,
    ) -> Option<bool> {
        let raw = std::fs::read_to_string(settings_path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        value.get("compaction")?.get("enabled")?.as_bool()
    }

    /// Effective pi compaction state for a project (plan 080):
    /// the project-level `.pi/settings.json` overrides the global
    /// `~/.pi/agent/settings.json`, which defaults to `true` when
    /// neither sets the key (pi's own default — `settings-manager.js`:
    /// `compaction?.enabled ?? true`).
    pub fn effective_pi_compaction_enabled(
        project_settings: &std::path::Path,
        global_settings: &std::path::Path,
    ) -> bool {
        Self::read_pi_compaction_enabled(project_settings)
            .or_else(|| Self::read_pi_compaction_enabled(global_settings))
            .unwrap_or(true)
    }
}

impl AgentRunner for PiJsonAgentRunner {
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
        // Build CLI args from agent_config, then append --mode json.
        let base_args = ctx.agent_config.build_cli_args();
        let cli_args = Self::build_json_cli_args(&base_args);

        // CLI path resolved once at construction time.
        let cli_path = self.cli_path.clone();

        // Spawn the child process in its own process group so we can
        // kill the entire group (including child processes) on timeout.
        let child = unsafe {
            std::process::Command::new(&cli_path)
                .args(&cli_args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                })
                .spawn()
        };

        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PortError::CommandNotFound(format!(
                    "'{}': {}",
                    cli_path, e
                )));
            }
            Err(e) => {
                return Err(PortError::AgentExecutionFailed {
                    message: format!(
                        "failed to spawn '{}': {}",
                        cli_path, e
                    ),
                    session_id: None,
                });
            }
        };

        let child_pid = child.id() as i32;
        let strand_desc = ctx.strand_path.0.display().to_string();
        let effective_timeout = ctx.timeout.unwrap_or(self.timeout);

        // Plan 081: shared liveness state — drained output buffers,
        // last-activity timestamp (stamped at spawn, so the
        // pre-first-byte window counts), and the watchdog's kill
        // reason.
        let live = LiveOutput::new();

        // Shared flag: set once the child has exited so the watchdog
        // suppresses its kill + warning.
        let cancelled = Arc::new(AtomicBool::new(false));

        // Reader threads drain stdout/stderr while the child runs —
        // any byte resets the inactivity timer (byte-level stall
        // detection, plan 081).
        let stdout_reader =
            spawn_reader(
                "json-stdout",
                child.stdout.take().expect("stdout was piped"),
                &live.stdout,
                &live.last_activity,
            )
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to spawn stdout reader: {e}"),
                    session_id: None,
                }
            })?;
        let stderr_reader =
            spawn_reader(
                "json-stderr",
                child.stderr.take().expect("stderr was piped"),
                &live.stderr,
                &live.last_activity,
            )
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to spawn stderr reader: {e}"),
                    session_id: None,
                }
            })?;

        // Watchdog thread: polls every 250 ms against the inactivity
        // window (checked first — the more specific diagnosis) and the
        // total budget; kills the process group on either deadline.
        let _watchdog = spawn_watchdog(
            "json-watchdog",
            child_pid,
            cli_path.clone(),
            strand_desc.clone(),
            effective_timeout,
            self.inactivity_timeout,
            &live,
            Arc::clone(&cancelled),
        )
        .map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!("failed to spawn watchdog thread: {e}"),
                session_id: None,
            }
        })?;

        // Write the prompt to the child's stdin.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let profile_prompt = ctx.profile_prompt.clone();
        let prompt_with_context =
            Self::build_prompt_with_context(&ctx, &profile_prompt);

        stdin
            .write_all(prompt_with_context.as_bytes())
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to write prompt to stdin: {e}"),
                    session_id: None,
                }
            })?;
        drop(stdin);

        // Wait for the child to exit and the readers to drain.
        // Use a thread + 2×-deadline join guard so we don't block
        // forever if an orphaned grandchild keeps a pipe open (the
        // `wait_with_output` hazard, preserved).
        let wait_handle = std::thread::Builder::new()
            .name("json-wait".to_string())
            .spawn(move || child.wait())
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to spawn wait thread: {e}"),
                    session_id: None,
                }
            })?;

        // Wait up to 2x the effective timeout for the child to exit
        // and the readers to drain; the join guard force-kills the
        // process group at the deadline.
        let wait_deadline = effective_timeout.saturating_mul(2)
            .max(Duration::from_secs(5));
        let status = join_all(
            wait_handle,
            stdout_reader,
            stderr_reader,
            child_pid,
            wait_deadline,
            &cancelled,
        )
        .map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!("failed to wait for '{cli_path}': {e}"),
                session_id: None,
            }
        })?;

        let stdout_bytes =
            live.stdout.lock().expect("stdout buffer poisoned").clone();
        let stderr_bytes =
            live.stderr.lock().expect("stderr buffer poisoned").clone();
        let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
        let raw_stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();

        // If status code is None, the process was killed by a signal
        // (SIGKILL from the watchdog, or an external signal).
        if status.code().is_none() {
            // Plan 081: classify by the watchdog's kill reason — the
            // child lost the race to a deadline. `None` is an
            // external signal kill: keep the legacy Timeout shape.
            let reason = *live.kill_reason.lock().expect("kill reason poisoned");
            let (
                _had_error,
                session_id,
                _response,
                _token_usage,
                _compactions,
                _error_message,
            ) = Self::parse_stdout(&raw_stdout);
            match reason {
                Some(KillReason::Inactivity) => {
                    let blocked_call = Self::parse_blocked_call(&raw_stdout);
                    let silent_secs = live.silence().as_secs();
                    let window_secs = self
                        .inactivity_timeout
                        .map(|w| w.as_secs())
                        .unwrap_or(0);
                    let blocked_part = blocked_call
                        .as_deref()
                        .map(|b| format!(", last call: {b}"))
                        .unwrap_or_default();
                    return Err(PortError::AgentInactivity {
                        message: format!(
                            "no output for {silent_secs}s (inactivity window {window_secs}s){blocked_part} ({cli_path}, strand: {strand_desc})"
                        ),
                        silent_secs,
                        window_secs,
                        blocked_call,
                        session_id,
                    });
                }
                Some(KillReason::Total) | None => {
                    return Err(PortError::Timeout {
                        message: format!(
                            "'{}' exceeded timeout of {:?} (strand: {})",
                            cli_path, effective_timeout, strand_desc
                        ),
                        session_id,
                    });
                }
            }
        }

        let exit_code = status.code().unwrap_or(-1);

        if exit_code != 0 {
            let (
                _had_error,
                session_id,
                _response,
                _token_usage,
                _compactions,
                _error_message,
            ) = Self::parse_stdout(&raw_stdout);
            return Err(PortError::AgentExecutionFailed {
                message: format!(
                    "'{}' exited with code {}: {}",
                    cli_path,
                    exit_code,
                    if stderr.is_empty() {
                        raw_stdout.clone()
                    } else {
                        stderr.clone()
                    }
                ),
                session_id,
            });
        }

        // Success — parse JSON-L from stdout.
        if raw_stdout.trim().is_empty() {
            return Ok(AgentOutput {
                stdout: String::new(),
                stderr,
                exit_code,
                metadata: None,
            });
        }

        let (had_parse_error, session_id, response_text, token_usage, compactions, error_message) =
            Self::parse_stdout(&raw_stdout);

        if had_parse_error {
            // Graceful degradation — treat as plain text.
            return Ok(AgentOutput {
                stdout: raw_stdout,
                stderr,
                exit_code,
                metadata: None,
            });
        }

        // Plan 079: terminal overflow — pi's own compact-and-retry
        // ran and the context still does not fit, and no final
        // response survived the stopReason filter. Session-resume
        // re-entry cannot help; fail fast instead of clocking up
        // the retries.
        //
        // Plan 080: overflow without compaction — when pi's
        // compaction never ran (disabled in pi settings), the stream
        // carries no compaction_end events at all: the only overflow
        // signal is the provider's error message on the
        // stopReason:"error" message. The same fail-fast applies —
        // the nudge loop re-enters the same over-full session and
        // cannot succeed (each nudge adds tokens).
        if response_text.trim().is_empty() {
            if let Some(rec) = Self::terminal_overflow(&compactions) {
                return Err(PortError::ContextLimitReached {
                    message: rec.error.clone().unwrap_or_else(|| {
                        "session context cannot fit the model window even after compaction".to_string()
                    }),
                    session_id,
                });
            }
            if let Some(ref err_msg) = error_message {
                if Self::is_context_overflow_message(err_msg) {
                    return Err(PortError::ContextLimitReached {
                        message: format!(
                            "context overflow, but pi auto-compaction did \
                             not run ({err_msg}). Enable compaction for \
                             rig sessions with a project-level \
                             .pi/settings.json: \
                             {{\"compaction\": {{\"enabled\": true}}}}",
                        ),
                        session_id,
                    });
                }
            }
        }

        Ok(AgentOutput {
            stdout: response_text,
            stderr,
            exit_code,
            metadata: Some(AgentInvocationMetadata {
                session_id,
                token_usage,
                compactions,
            }),
        })
    }

    fn execute_with_config(
        &self,
        agent_config: &AgentConfig,
        strand_path: StrandPath,
        strand_file_ref: Option<StrandPath>,
        prompt: String,
        profile_prompt: String,
        event_type: String,
        knot_name: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<AgentOutput, PortError> {
        // Clone config and append adapter-specific args via extra_args.
        // The retry loop populates extra_args with --session-id.
        let mut config = agent_config.clone();

        // Append --name for pi session title.
        let strand_filename = strand_path.0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_title = format!(
            "{} triggered by {} on {}",
            knot_name.as_deref().unwrap_or("unknown"),
            event_type,
            strand_filename,
        );
        config.extra_args.push("--name".to_string());
        config.extra_args.push(session_title);
        // Append strand content reference using pi's @file syntax.
        // Only for Created/Modified events (file exists on disk).
        if let Some(ref file_path) = strand_file_ref {
            config.extra_args.push(format!("@{}", file_path.0.display()));
        }

        let ctx = ExecutionContext {
            agent_config: config,
            prompt,
            profile_prompt,
            strand_path,
            event_type,
            knot_name,
            timeout,
        };
        self.execute(ctx)
    }

    fn runner_type(&self) -> &str {
        "pi-json"
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Mock script: passes stdin through to stdout (for JSON-L parsing tests).
    fn make_json_mock_script() -> String {
        r#"#!/usr/bin/env bash
cat
echo ""
"#
            .to_string()
    }

    /// Create a unique temp directory for mock scripts, returning the path
    /// to the mock binary and the `TempDir` handle (caller must keep alive).
    fn make_json_mock_path() -> (PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mock-pi-json");
        (path, dir)
    }

    /// Create a PiJsonAgentRunner configured with the passthrough mock.
    /// Returns `(runner, tempdir)` — caller must keep `tempdir` alive for the
    /// duration of the test.
    fn make_mock_json_runner() -> (PiJsonAgentRunner, tempfile::TempDir) {
        let script = make_json_mock_script();
        let (path, dir) = make_json_mock_path();
        std::fs::write(&path, &script).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o755),
            )
            .ok();
        }
        (
            PiJsonAgentRunner::with_cli_path(path.to_string_lossy().to_string()),
            dir,
        )
    }

    /// Create a PiJsonAgentRunner whose mock binary emits the given
    /// JSON-L script (plan 079 compaction tests). Returns `(runner,
    /// tempdir)` — caller must keep `tempdir` alive.
    fn make_json_emitting_runner(script: &str) -> (PiJsonAgentRunner, tempfile::TempDir) {
        let (path, dir) = make_json_mock_path();
        std::fs::write(&path, script).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o755),
            )
            .ok();
        }
        (
            PiJsonAgentRunner::with_cli_path(path.to_string_lossy().to_string()),
            dir,
        )
    }

    /// Blocking mock script: sleeps for a long time.
    /// Used by timeout tests so the timeout thread actually fires.
    fn make_json_blocking_mock_script() -> String {
        r#"#!/usr/bin/env bash
sleep 300
"#
            .to_string()
    }

    /// Create a unique temp directory for the blocking mock, returning the
    /// path and the `TempDir` handle (caller must keep alive).
    fn make_json_blocking_mock_path() -> (PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mock-pi-json-blocking");
        (path, dir)
    }

    /// Create a PiJsonAgentRunner configured with the blocking mock.
    /// The process stays alive long enough for timeout tests to fire.
    /// Returns `(runner, tempdir)` — caller must keep `tempdir` alive.
    fn make_blocking_json_runner() -> (PiJsonAgentRunner, tempfile::TempDir) {
        let script = make_json_blocking_mock_script();
        let (path, dir) = make_json_blocking_mock_path();
        std::fs::write(&path, &script).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o755),
            )
            .ok();
        }
        (
            PiJsonAgentRunner::with_cli_path(path.to_string_lossy().to_string()),
            dir,
        )
    }

    fn make_context(args: &[&str]) -> ExecutionContext {
        ExecutionContext {
            agent_config: AgentConfig {
                goal: "test".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: args.iter().map(|s| s.to_string()).collect(),
                thinking_level: None,
            },
            prompt: "test prompt".to_string(),
            profile_prompt: "You are a test agent.".to_string(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }


    // ── JSON-L Parser Unit Tests (direct, no subprocess) ─────────────

    /// Helper to call parse_stdout from tests.
    fn run_parse_stdout(
        raw: &str,
    ) -> (
        bool,
        Option<String>,
        String,
        Option<TokenUsage>,
        Vec<CompactionRecord>,
        Option<String>,
    ) {
        PiJsonAgentRunner::parse_stdout(raw)
    }

    /// Unit test: `parse_stdout` extracts session ID from JSON-L.
    #[test]
    fn test_json_runner_parses_session_id() {
        let raw = r#"{"type":"session","id":"abc-123"}
{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"hello"}]}]}"#;
        let (had_error, session_id, response_text, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(!had_error, "should parse cleanly");
        assert_eq!(session_id.as_deref(), Some("abc-123"));
        assert!(response_text.contains("hello"));
    }

    /// Unit test: `parse_stdout` extracts token usage.
    #[test]
    fn test_json_runner_parses_token_usage() {
        let raw = r#"{"type":"session","id":"sess-1"}
{"type":"agent_end","usage":{"input":100,"output":50,"cache_read":10,"cache_write":5,"total":165},"messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"ok"}]}]}"#;
        let (_had_error, _session_id, _response, usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        let usage = usage.unwrap();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 50);
        assert_eq!(usage.cache_read, 10);
        assert_eq!(usage.cache_write, 5);
        assert_eq!(usage.total, 165);
    }

    /// Unit test: `parse_stdout` extracts response text.
    #[test]
    fn test_json_runner_parses_response_text() {
        let raw = r#"{"type":"session","id":"sess-x"}
{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"the response text"}]}]}"#;
        let (_had_error, _session_id, response_text, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(response_text.contains("the response text"));
    }

    /// Unit test: `parse_stdout` extracts session_id from timeout error.
    #[test]
    fn test_json_runner_timeout_captures_session_id() {
        let raw = r#"{"type":"session","id":"timeout-sess"}"#;
        let (_had_error, session_id, _response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert_eq!(session_id.as_deref(), Some("timeout-sess"));
    }

    /// Unit test: `parse_stdout` extracts session_id from failure output.
    #[test]
    fn test_json_runner_nonzero_exit_captures_session_id() {
        let raw = r#"{"type":"session","id":"fail-sess"}"#;
        let (_had_error, session_id, _response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert_eq!(session_id.as_deref(), Some("fail-sess"));
    }

    #[test]
    fn test_json_runner_command_not_found() {
        let runner = PiJsonAgentRunner::with_cli_path(
            "/nonexistent/json-runner".to_string(),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for missing binary");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::CommandNotFound(_)),
            "expected CommandNotFound, got {err:?}"
        );
        assert!(
            err.session_id().is_none(),
            "no session_id for CommandNotFound"
        );
    }

    /// Unit test: malformed JSON sets had_error flag.
    /// The raw fallback (returning full stdout) happens at the
    /// `execute` level when had_parse_error is true.
    #[test]
    fn test_json_runner_malformed_json_fallback() {
        let raw = "not json at all\ngarbled output\n";
        let (had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(had_error, "should have parse errors");
        // parse_stdout doesn't accumulate raw lines — response_text
        // is empty for non-JSON input. The caller (execute) uses the
        // had_error flag to fall back to raw stdout instead.
        assert!(response.is_empty());
    }

    /// Unit test: empty input produces empty output.
    #[test]
    fn test_json_runner_empty_output() {
        let raw = "";
        let (had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(!had_error);
        assert!(response.is_empty());
    }

    /// Unit test: `--mode json` is appended by `build_json_cli_args`.
    #[test]
    fn test_json_runner_adds_mode_json_flag() {
        let base_args = vec!["-p".to_string(), "--model".to_string(), "gpt-4o".to_string()];
        let json_args = PiJsonAgentRunner::build_json_cli_args(&base_args);
        assert!(json_args.contains(&"--mode".to_string()));
        assert!(json_args.contains(&"json".to_string()));
        assert!(json_args.contains(&"gpt-4o".to_string()));
    }

    /// Unit test: `message_end` events are now ignored.
    /// The message_end handler was removed — agent_end has the complete
    /// message array with stopReason filtering.
    #[test]
    fn test_json_runner_parses_message_end_response() {
        let raw = r#"{"type":"session","id":"msg-sess"}
{"type":"message_end","role":"assistant","content":"response from message_end"}"#;
        let (_had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        // message_end no longer extracts text — response should be empty
        assert!(response.is_empty());
    }

    /// Unit test: tool-use messages are excluded by stopReason filter.
    /// agent_end with two assistant messages: one toolUse (intermediate)
    /// and one stop (final). Only the final message text appears.
    #[test]
    fn test_json_runner_excludes_tool_use_messages() {
        let raw = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"Let me check the file..."}]},{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"The file contains 42 lines."}]}]}"#;
        let (_had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(response.contains("42 lines"), "should contain final response");
        assert!(
            !response.contains("Let me check"),
            "should not contain intermediate tool-use message: {}",
            response
        );
    }

    /// Unit test: stopReason "length" (truncated) is included.
    /// Truncated responses are still the agent's final output.
    #[test]
    fn test_json_runner_includes_length_stop_reason() {
        let raw = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"length","content":[{"type":"text","text":"truncated response"}]}]}"#;
        let (_had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(response.contains("truncated response"));
    }

    /// Unit test: stopReason "error" is excluded.
    /// Error conditions should not produce response text.
    #[test]
    fn test_json_runner_excludes_error_stop_reason() {
        let raw = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error","content":[{"type":"text","text":"error output"}]}]}"#;
        let (_had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(response.is_empty());
    }

    /// Plan 079: a successful `compaction_end` is recorded in stream
    /// order with the pre-compaction token count from `result`.
    #[test]
    fn test_json_runner_parses_compaction_end() {
        let raw = r#"{"type":"session","id":"sess-compact"}
{"type":"compaction_end","reason":"overflow","result":{"summary":"s","firstKeptEntryId":"e","tokensBefore":150000,"details":{}},"aborted":false,"willRetry":true}
{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"done"}]}]}"#;
        let (_had_error, _session_id, _response, _usage, compactions, _error_message) =
            run_parse_stdout(raw);
        assert_eq!(compactions.len(), 1, "one compaction recorded");
        assert_eq!(compactions[0].reason, "overflow");
        assert_eq!(compactions[0].tokens_before, Some(150000));
        assert!(compactions[0].will_retry);
        assert!(compactions[0].error.is_none());
    }

    /// Plan 079: a failed `compaction_end` (no `result` — pi omits the
    /// key on failure) records the error message and no token count; the
    /// `stopReason: "error"` final message is excluded, so the response
    /// text is empty.
    #[test]
    fn test_json_runner_parses_compaction_end_failed() {
        let raw = r#"{"type":"session","id":"sess-compact-fail"}
{"type":"compaction_end","reason":"overflow","aborted":false,"willRetry":false,"errorMessage":"Context overflow recovery failed after one compact-and-retry attempt."}
{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error","content":[{"type":"text","text":"context overflow"}]}]}"#;
        let (_had_error, _session_id, response, _usage, compactions, _error_message) =
            run_parse_stdout(raw);
        assert_eq!(compactions.len(), 1, "one compaction recorded");
        assert_eq!(compactions[0].reason, "overflow");
        assert!(!compactions[0].will_retry);
        assert_eq!(
            compactions[0].tokens_before,
            None,
            "no result on failed compaction → no token count"
        );
        assert_eq!(
            compactions[0].error.as_deref(),
            Some("Context overflow recovery failed after one compact-and-retry attempt."),
            "errorMessage should be captured"
        );
        assert!(
            response.is_empty(),
            "error stopReason is excluded from response: {response}"
        );
    }

    /// Plan 079: `compaction_start` carries no data of interest — the end
    /// event carries everything — so it produces no record.
    #[test]
    fn test_json_runner_ignores_compaction_start() {
        let raw = r#"{"type":"session","id":"sess-compact-start"}
{"type":"compaction_start","reason":"overflow"}
{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"ok"}]}]}"#;
        let (_had_error, _session_id, _response, _usage, compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(
            compactions.is_empty(),
            "compaction_start must not produce a record"
        );
    }

    /// Unit test: multiple tool-use messages followed by a final stop.
    /// Only the final (stop) message text appears in response.
    #[test]
    fn test_json_runner_multiple_tool_use_then_stop() {
        let raw = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"Checking config..."}]},{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"Reading database..."}]},{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Config and database are in sync."}]}]}"#;
        let (_had_error, _session_id, response, _usage, _compactions, _error_message) =
            run_parse_stdout(raw);
        assert!(response.contains("in sync"), "should contain final response");
        assert!(
            !response.contains("Checking config"),
            "should not contain first tool-use message: {}",
            response
        );
        assert!(
            !response.contains("Reading database"),
            "should not contain second tool-use message: {}",
            response
        );
    }

    /// Plan 079: a terminal overflow — recovery ran (`willRetry: true`)
    /// and the context overflowed again (`willRetry: false`) — is
    /// classified as `PortError::ContextLimitReached` carrying the
    /// session ID and pi's error message.
    #[test]
    fn test_json_runner_terminal_overflow_returns_context_limit_reached() {
        let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-ctx"}'
echo '{"type":"compaction_end","reason":"overflow","result":{"summary":"s","firstKeptEntryId":"e","tokensBefore":150000,"details":{}},"aborted":false,"willRetry":true}'
echo '{"type":"compaction_end","reason":"overflow","aborted":false,"willRetry":false,"errorMessage":"Context overflow recovery failed after one compact-and-retry attempt. The session context is still too large."}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error","content":[{"type":"text","text":"context overflow"}]}]}'
exit 0
"#;
        let (runner, _dir) = make_json_emitting_runner(script);
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(
            result.is_err(),
            "terminal overflow should fail, got: {result:?}"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::ContextLimitReached { .. }),
            "expected ContextLimitReached, got {err:?}"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-ctx"),
            "error should carry the session ID"
        );
        assert!(
            err.to_string().contains("Context overflow recovery failed"),
            "message should carry pi's errorMessage, got: {err}"
        );
    }

    /// Plan 079: a recovered overflow — compaction ran, pi retried
    /// in-process, the turn ended with a normal final response — is a
    /// plain success: the response text is returned and the compaction
    /// is recorded in the metadata.
    #[test]
    fn test_json_runner_overflow_recovered_is_success() {
        let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-ctx-ok"}'
echo '{"type":"compaction_end","reason":"overflow","result":{"summary":"s","firstKeptEntryId":"e","tokensBefore":150000,"details":{}},"aborted":false,"willRetry":true}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"done after compact"}]}]}'
exit 0
"#;
        let (runner, _dir) = make_json_emitting_runner(script);
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(
            result.is_ok(),
            "recovered overflow should succeed: {result:?}"
        );
        let output = result.unwrap();
        assert!(
            output.stdout.contains("done after compact"),
            "stdout should contain the final response: {}",
            output.stdout
        );
        let metadata = output.metadata.expect("metadata should be present");
        assert_eq!(
            metadata.compactions.len(),
            1,
            "one compaction recorded in metadata"
        );
        assert_eq!(metadata.compactions[0].reason, "overflow");
        assert_eq!(metadata.compactions[0].tokens_before, Some(150000));
        assert!(metadata.compactions[0].will_retry);
        assert!(metadata.compactions[0].error.is_none());
    }

    // ── Plan 080: overflow without compaction (fail-fast) ──────

    /// Plan 080: `parse_stdout` captures the provider error message of
    /// a `stopReason: "error"` assistant message from `agent_end`.
    #[test]
    fn test_json_runner_parses_error_message() {
        let raw = r#"{"type":"session","id":"sess-err"}
{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error","content":[],"errorMessage":"400 request (200287 tokens) exceeds the available context size (200192 tokens), try increasing it"}]}"#;
        let (_had_error, _session_id, response, _usage, _compactions, error_message) =
            run_parse_stdout(raw);
        assert!(
            response.is_empty(),
            "error stopReason is excluded from response: {response}"
        );
        assert_eq!(
            error_message.as_deref(),
            Some(
                "400 request (200287 tokens) exceeds the available context size (200192 tokens), try increasing it"
            ),
            "errorMessage should be captured, got: {error_message:?}"
        );
    }

    /// Plan 080: a `stopReason: "stop"` message with no errorMessage
    /// leaves the captured error `None` (no error to classify).
    #[test]
    fn test_json_runner_no_error_message_on_success() {
        let raw = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"ok"}]}]}"#;
        let (_had_error, _session_id, _response, _usage, _compactions, error_message) =
            run_parse_stdout(raw);
        assert!(error_message.is_none(), "no errorMessage on a clean turn: {error_message:?}");
    }

    /// Plan 080: the borrow-my-stuff shape — compaction disabled in pi
    /// settings, so the stream has **no** compaction_end events; the
    /// overflow surfaces only as the provider's 400 on the error
    /// message. Classified as `ContextLimitReached` (fail-fast) with
    /// the provider message and the settings hint, not an empty
    /// response for the nudge loop.
    #[test]
    fn test_json_runner_overflow_error_without_compaction_is_context_limit() {
        let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-nocomp"}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error","content":[],"errorMessage":"400 request (216476 tokens) exceeds the available context size (200192 tokens), try increasing it"}]}'
exit 0
"#;
        let (runner, _dir) = make_json_emitting_runner(script);
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(
            result.is_err(),
            "overflow error without compaction should fail, got: {result:?}"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::ContextLimitReached { .. }),
            "expected ContextLimitReached, got {err:?}"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-nocomp"),
            "error should carry the session ID"
        );
        assert!(
            err.to_string().contains("exceeds the available context size"),
            "message should carry pi's provider errorMessage, got: {err}"
        );
        assert!(
            err.to_string().contains(".pi/settings.json"),
            "message should hint at the settings fix, got: {err}"
        );
        assert!(
            err.to_string().contains("compaction did not run"),
            "message should state compaction never ran, got: {err}"
        );
    }

    /// Plan 080: a non-overflow provider error (rate limiting) on the
    /// error message is **not** `ContextLimitReached` — the turn ends
    /// with empty response text and the nudge loop keeps its job
    /// (transient errors can recover).
    #[test]
    fn test_json_runner_non_overflow_error_without_compaction_stays_ok() {
        let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-ratelimit"}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error","content":[],"errorMessage":"429 rate limit exceeded: too many requests, please retry later"}]}'
exit 0
"#;
        let (runner, _dir) = make_json_emitting_runner(script);
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(
            result.is_ok(),
            "non-overflow error should not fail fast: {result:?}"
        );
        assert!(
            result.unwrap().stdout.trim().is_empty(),
            "error stopReason is excluded from the response"
        );
    }

    /// Plan 080: the overflow classifier matches the provider
    /// signatures and rejects non-overflow errors — including the
    /// Bedrock throttling message that contains "Too many tokens".
    #[test]
    fn test_is_context_overflow_message_classification() {
        // Positive — the real signature from the borrow-my-stuff incident
        // (llama.cpp server) plus the other common providers.
        assert!(PiJsonAgentRunner::is_context_overflow_message(
            "400 request (200287 tokens) exceeds the available context size (200192 tokens), try increasing it"
        ));
        assert!(PiJsonAgentRunner::is_context_overflow_message(
            "prompt is too long: 213462 tokens > 200000 maximum"
        ));
        assert!(PiJsonAgentRunner::is_context_overflow_message(
            "Your input exceeds the context window of this model"
        ));
        assert!(PiJsonAgentRunner::is_context_overflow_message(
            "Requested token count exceeds the model's maximum context length of 131072 tokens"
        ));
        assert!(PiJsonAgentRunner::is_context_overflow_message(
            "The input token count (1196265) exceeds the maximum number of tokens allowed (1048575)"
        ));
        assert!(PiJsonAgentRunner::is_context_overflow_message(
            "413 {\"error\":{\"type\":\"request_too_large\"}}"
        ));
        // Negative — non-overflow provider errors.
        assert!(!PiJsonAgentRunner::is_context_overflow_message(
            "429 rate limit exceeded: too many requests"
        ));
        assert!(!PiJsonAgentRunner::is_context_overflow_message(
            "ThrottlingException: Too many tokens, please wait before trying again."
        ));
        assert!(!PiJsonAgentRunner::is_context_overflow_message(
            "401 invalid api key"
        ));
        assert!(!PiJsonAgentRunner::is_context_overflow_message(
            "500 internal server error"
        ));
        assert!(!PiJsonAgentRunner::is_context_overflow_message(""));
    }

    /// Plan 080: effective compaction resolution — project-level
    /// `.pi/settings.json` overrides global; pi's default is enabled.
    #[test]
    fn test_effective_pi_compaction_enabled_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(".pi").join("settings.json");
        let global = dir.path().join("global-settings.json");
        let absent = dir.path().join("absent.json");

        // Both absent → pi's default: enabled.
        assert!(PiJsonAgentRunner::effective_pi_compaction_enabled(&absent, &absent));

        // Global disabled, project absent → disabled (the
        // borrow-my-stuff shape before the fix).
        std::fs::write(&global, r#"{"compaction": {"enabled": false}}"#).unwrap();
        assert!(!PiJsonAgentRunner::effective_pi_compaction_enabled(&absent, &global));

        // Project enabled overrides global disabled.
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        std::fs::write(&project, r#"{"compaction": {"enabled": true}}"#).unwrap();
        assert!(PiJsonAgentRunner::effective_pi_compaction_enabled(&project, &global));

        // Project file without the compaction key falls through to global.
        std::fs::write(&project, r#"{"theme": "dark"}"#).unwrap();
        assert!(!PiJsonAgentRunner::effective_pi_compaction_enabled(&project, &global));

        // Unparseable project settings fall through to global.
        std::fs::write(&project, "not json").unwrap();
        assert!(!PiJsonAgentRunner::effective_pi_compaction_enabled(&project, &global));

        // Project disabled is honoured (explicit operator choice).
        std::fs::write(&project, r#"{"compaction": {"enabled": false}}"#).unwrap();
        assert!(!PiJsonAgentRunner::effective_pi_compaction_enabled(&project, &global));
    }

    /// Integration test: mock echoes stdin which contains the prompt.
    #[test]
    fn test_json_runner_prompt_passthrough() {
        let (runner, _dir) = make_mock_json_runner();
        let mut ctx = make_context(&[]);
        ctx.prompt = "my knot instructions".to_string();

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // The mock echoes stdin, which contains the prompt chain.
        assert!(
            output.stdout.contains("my knot instructions"),
            "stdout should contain prompt: {}",
            output.stdout
        );
    }

    /// Integration test: mock echoes stdin → timeout kills it.
    #[test]
    fn test_json_runner_context_timeout_override() {
        let (runner, _dir) = make_blocking_json_runner();
        let mut ctx = make_context(&[]);
        ctx.timeout = Some(Duration::from_millis(50));

        let start = std::time::Instant::now();
        let result = runner.execute(ctx);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should error for timeout");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "should use context timeout, not runner default"
        );
    }

    // ── Plan 081: inactivity watchdog ────────────────────────────────

    /// Plan 081: the most recent `tool_execution_start` with no
    /// matching end is named `{toolName}({args preview})` — the
    /// `command` field when present. Malformed lines are ignored.
    #[test]
    fn test_parse_blocked_call_open_tool_is_named() {
        let raw = "not json at all\n\
                       {\"type\":\"session\",\"id\":\"sess-blocked\"}\n\
                       {\"type\":\"tool_execution_start\",\"toolCallId\":\"t1\",\"toolName\":\"bash\",\"args\":{\"command\":\"npm run build\"}}";
        assert_eq!(
            PiJsonAgentRunner::parse_blocked_call(raw),
            Some("bash(npm run build)".to_string())
        );
    }

    /// Plan 081: a matching `tool_execution_end` closes the call → no
    /// blocked call.
    #[test]
    fn test_parse_blocked_call_matched_end_is_none() {
        let raw = "{\"type\":\"tool_execution_start\",\"toolCallId\":\"t1\",\"toolName\":\"bash\",\"args\":{\"command\":\"npm run build\"}}\n\
                   {\"type\":\"tool_execution_end\",\"toolCallId\":\"t1\",\"content\":\"done\"}";
        assert_eq!(
            PiJsonAgentRunner::parse_blocked_call(raw),
            None
        );
    }

    /// Plan 081: with multiple open calls, the LAST open one is the
    /// blocked call.
    #[test]
    fn test_parse_blocked_call_multiple_starts_last_open() {
        let raw = "{\"type\":\"tool_execution_start\",\"toolCallId\":\"t1\",\"toolName\":\"bash\",\"args\":{\"command\":\"first\"}}\n\
                   {\"type\":\"tool_execution_start\",\"toolCallId\":\"t2\",\"toolName\":\"read\",\"args\":{\"path\":\"/tmp/x\"}}";
        assert_eq!(
            PiJsonAgentRunner::parse_blocked_call(raw),
            Some("read({\"path\":\"/tmp/x\"})".to_string())
        );
    }

    /// Plan 081: matching is by `toolCallId` — an earlier call ending
    /// keeps the later open call blocked.
    #[test]
    fn test_parse_blocked_call_earlier_end_keeps_later_open() {
        let raw = "{\"type\":\"tool_execution_start\",\"toolCallId\":\"t1\",\"toolName\":\"bash\",\"args\":{\"command\":\"first\"}}\n\
                   {\"type\":\"tool_execution_start\",\"toolCallId\":\"t2\",\"toolName\":\"read\",\"args\":{\"path\":\"/tmp/x\"}}\n\
                   {\"type\":\"tool_execution_end\",\"toolCallId\":\"t1\"}";
        assert_eq!(
            PiJsonAgentRunner::parse_blocked_call(raw),
            Some("read({\"path\":\"/tmp/x\"})".to_string())
        );
    }

    /// Plan 081: args without a `command` field fall back to the first
    /// 80 chars of the args JSON (truncation marked).
    #[test]
    fn test_parse_blocked_call_args_preview_truncated() {
        let long = "x".repeat(100);
        let raw = format!(
            "{{\"type\":\"tool_execution_start\",\"toolCallId\":\"t1\",\"toolName\":\"edit\",\"args\":{{\"path\":\"{long}\"}}}}"
        );
        let label =
            PiJsonAgentRunner::parse_blocked_call(&raw).expect("open call named");
        assert!(label.starts_with("edit({\"path\":\"xxx"), "label: {label}");
        // `edit(` + 80 chars of JSON preview + `…` + `)`
        assert_eq!(
            label.chars().count(),
            "edit(".chars().count() + 80 + 1 + 1,
            "label: {label}"
        );
    }

    /// Plan 081: an empty stream has no blocked call.
    #[test]
    fn test_parse_blocked_call_empty_stream() {
        assert_eq!(PiJsonAgentRunner::parse_blocked_call(""), None);
    }

    /// Create a PiJsonAgentRunner with the given mock script, total
    /// timeout, and inactivity window (plan 081). Returns `(runner,
    /// tempdir)` — caller must keep `tempdir` alive.
    fn make_json_inactivity_runner(
        script: &str,
        total_timeout: Duration,
        inactivity_timeout: Option<Duration>,
    ) -> (PiJsonAgentRunner, tempfile::TempDir) {
        let (path, dir) = make_json_mock_path();
        std::fs::write(&path, script).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o755),
            )
            .ok();
        }
        (
            PiJsonAgentRunner::with_cli_path_and_timeouts(
                path.to_string_lossy().to_string(),
                total_timeout,
                inactivity_timeout,
            ),
            dir,
        )
    }

    /// Plan 081: a session that is silent for the inactivity window is
    /// killed by the watchdog → `AgentInactivity` with the captured
    /// session ID, the silence length (≥ window), and a
    /// `no output for` message. The total timeout is far away, so the
    /// kill is the inactivity watchdog, not the budget.
    #[test]
    fn execute_inactivity_kill() {
        let script = r#"#!/usr/bin/env bash
echo '{"type":"session","id":"sess-inact"}'
sleep 300
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_secs(30),
            Some(Duration::from_millis(200)),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for inactivity");
        match result.unwrap_err() {
            PortError::AgentInactivity {
                message,
                silent_secs,
                window_secs,
                session_id,
                ..
            } => {
                assert_eq!(session_id.as_deref(), Some("sess-inact"));
                assert!(
                    silent_secs >= window_secs,
                    "silent_secs ({silent_secs}) should be >= window_secs ({window_secs})"
                );
                assert!(
                    message.contains("no output for"),
                    "message should contain 'no output for': {message}"
                );
            }
            other => panic!("expected AgentInactivity, got: {other:?}"),
        }
    }

    /// Plan 081: with the watchdog disabled (inactivity `None`), a silent
    /// session falls to the **total** timeout — the two watchdogs are
    /// distinct and disabling inactivity does not remove the budget.
    #[test]
    fn execute_inactivity_disabled() {
        let script = r#"#!/usr/bin/env bash
echo '{"type":"session","id":"sess-inact-off"}'
sleep 300
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_millis(500),
            None,
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        let err = result
            .expect_err("should error (total timeout, not inactivity)");
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "with inactivity disabled the total timeout should fire, got: {err:?}"
        );
    }

    /// Plan 081: steady output keeps the watchdog quiet — a line every
    /// 100 ms for ~2 s against a 200 ms window → the session completes
    /// normally (the timer kept resetting on every byte).
    #[test]
    fn execute_inactivity_reset_by_output() {
        let script = r#"#!/usr/bin/env bash
for i in $(seq 1 20); do
  echo '{"type":"tool_execution_update","toolCallId":"t1","content":"progress $i"}'
  sleep 0.1
done
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_secs(15),
            Some(Duration::from_millis(200)),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(
            result.is_ok(),
            "an outputting session should complete (timer kept resetting): {result:?}"
        );
    }

    /// Plan 081: output resets the timer continuously — one line, then
    /// silence. The kill proves the reset is per-byte (not just the
    /// pre-first-byte spawn window), and a non-session stream still
    /// yields `AgentInactivity` (no session ID captured).
    #[test]
    fn execute_inactivity_reset_then_stall() {
        let script = r#"#!/usr/bin/env bash
echo '{"type":"tool_execution_update","toolCallId":"t1","content":"progress 1"}'
sleep 300
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_secs(30),
            Some(Duration::from_millis(200)),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for inactivity");
        match result.unwrap_err() {
            PortError::AgentInactivity { session_id, .. } => {
                assert!(
                    session_id.is_none(),
                    "no session line was emitted — no session ID"
                );
            }
            other => panic!("expected AgentInactivity, got: {other:?}"),
        }
    }

    /// Plan 081: the blocked call is named in the message — the most
    /// recent `tool_execution_start` without a matching end.
    #[test]
    fn execute_inactivity_names_blocked_tool() {
        let script = r#"#!/usr/bin/env bash
echo '{"type":"session","id":"sess-blocked"}'
echo '{"type":"tool_execution_start","toolCallId":"t1","toolName":"bash","args":{"command":"npm run build"}}'
sleep 300
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_secs(30),
            Some(Duration::from_millis(200)),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for inactivity");
        let err = result.unwrap_err();
        match &err {
            PortError::AgentInactivity { message, .. } => {
                assert!(
                    message.contains("bash"),
                    "message should name the blocked tool: {message}"
                );
                assert!(
                    message.contains("npm run build"),
                    "message should carry the command preview: {message}"
                );
            }
            other => panic!("expected AgentInactivity, got: {other:?}"),
        }
    }

    /// Plan 081: silence from spawn (no output at all, not even a
    /// session line) → `AgentInactivity { session_id: None }` — the
    /// pre-first-byte window counts.
    #[test]
    fn execute_inactivity_before_session_line() {
        let script = r#"#!/usr/bin/env bash
sleep 300
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_secs(30),
            Some(Duration::from_millis(200)),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for inactivity");
        match result.unwrap_err() {
            PortError::AgentInactivity { session_id, .. } => {
                assert!(
                    session_id.is_none(),
                    "no session line was emitted — no session ID"
                );
            }
            other => panic!("expected AgentInactivity, got: {other:?}"),
        }
    }

    /// Plan 081 regression: silent script, inactivity **disabled**, small
    /// total → `PortError::Timeout` — the two watchdogs are distinct.
    #[test]
    fn execute_total_timeout_still_timeout() {
        let script = r#"#!/usr/bin/env bash
sleep 300
"#;
        let (runner, _dir) = make_json_inactivity_runner(
            script,
            Duration::from_millis(500),
            None,
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        let err = result.expect_err("should error (total timeout)");
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got: {err:?}"
        );
    }
}
