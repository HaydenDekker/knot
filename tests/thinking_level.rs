//! Acceptance tests for thinking-level (074 phase 5).
//!
//! Full flow through the **real** adapters: a `model-ref` profile parsed
//! from frontmatter, resolved against a real `models.yml` (parsed with
//! `ModelRegistry::from_yaml`) that carries an alias default, with the
//! profile overriding it. The resolved config is executed through both
//! `PiStdioAgentRunner` and `PiJsonAgentRunner` against a mock CLI that
//! records its argv; we assert the spawned argv carries
//! `--thinking <effective-level>` in both — closing the gap that no test
//! previously proved a registry/profile `thinking-level` reaches the
//! spawned `pi` argv.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use knot::application::ports::{AgentRunner, ExecutionContext};
use knot::application::usecases::test_fixtures::KnotBuilder;
use knot::domain::entities::StrandPath;
use knot::domain::knot_file::parse_agent_profile;
use knot::domain::value_objects::{AgentConfig, ModelRegistry, ThinkingLevel};
use knot::{PiJsonAgentRunner, PiStdioAgentRunner};
use tempfile::TempDir;

/// Create a mock CLI script in `dir` that records its argv (one arg per
/// line) to `<dir>/argv.txt` and echoes stdin to stdout. Returns
/// `(script_path, argv_path)`.
fn make_argv_recording_mock(dir: &Path) -> (PathBuf, PathBuf) {
    fs::create_dir_all(dir).unwrap();
    let script = dir.join("mock-pi");
    let argv = dir.join("argv.txt");
    fs::write(
        &script,
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/argv.txt\"\ncat\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (script, argv)
}

/// Build the resolved `AgentConfig` for a `model-ref` profile that
/// **overrides** its alias's thinking-level default.
///
/// - `models.yml`: alias `frontier` carries `thinking-level: high` (default).
/// - profile: `model-ref: frontier` + `thinking-level: xhigh` (override).
///
/// The effective level must be `xhigh` (profile wins over alias).
fn resolved_config_with_override() -> AgentConfig {
    let models_yml = "models:\n  frontier:\n    provider: anthropic\n    model: claude-sonnet-4-20250514\n    thinking-level: high\n";
    let registry = ModelRegistry::from_yaml(models_yml).unwrap();

    let profile_md = "---\nname: analyst\nmodel-ref: frontier\nthinking-level: xhigh\n---\n\nYou are a deep analyst.\n";
    let profile = parse_agent_profile(profile_md).unwrap();
    assert_eq!(
        profile.thinking_level,
        Some(ThinkingLevel::XHigh),
        "profile frontmatter should parse the override level"
    );

    let knot = KnotBuilder::new("k1")
        .with_profile("analyst")
        .with_instructions("Analyze the strand.")
        .build();

    profile.resolve_for_knot(&knot, &registry).unwrap()
}

/// Run `make_runner` (pointed at a fresh argv-recording mock) against
/// `config`, then assert the spawned argv contains `--thinking xhigh`
/// placed after `--model`.
fn assert_thinking_in_spawned_argv<F>(
    make_runner: F,
    config: &AgentConfig,
    root: &Path,
) where
    F: Fn(&str) -> Box<dyn AgentRunner>,
{
    let (script, argv_path) = make_argv_recording_mock(root);
    let path_str = script.to_string_lossy().to_string();
    let runner = make_runner(&path_str);

    let ctx = ExecutionContext {
        agent_config: config.clone(),
        prompt: "Analyze the strand.".to_string(),
        profile_prompt: "You are a deep analyst.".to_string(),
        strand_path: StrandPath(PathBuf::from("strand.md")),
        event_type: "Created".to_string(),
        knot_name: Some("k1".to_string()),
        timeout: None,
    };
    let result = runner.execute(ctx);
    assert!(result.is_ok(), "runner should succeed: {result:?}");

    let argv = fs::read_to_string(&argv_path).unwrap_or_else(|e| {
        panic!("argv file missing at {}: {e}", argv_path.display())
    });
    let args: Vec<&str> = argv.lines().collect();

    let ti = args
        .iter()
        .position(|a| *a == "--thinking")
        .unwrap_or_else(|| panic!("argv should contain --thinking, got: {args:?}"));
    assert_eq!(
        args[ti + 1],
        "xhigh",
        "effective (profile-override) level should reach argv; argv: {args:?}"
    );

    // `--thinking` sits with the model options — after `--model`.
    let mi = args
        .iter()
        .position(|a| *a == "--model")
        .unwrap_or_else(|| panic!("argv should contain --model, got: {args:?}"));
    assert!(ti > mi, "--thinking should come after --model; argv: {args:?}");
}

/// The stdio runner's spawned argv carries the overridden thinking level.
#[test]
fn stdio_runner_spawned_argv_carries_overridden_thinking_level() {
    let tmp = TempDir::new().unwrap();
    let config = resolved_config_with_override();
    // Sanity: effective level is the profile override, not the alias default.
    assert_eq!(config.thinking_level, Some(ThinkingLevel::XHigh));

    assert_thinking_in_spawned_argv(
        |path| {
            Box::new(PiStdioAgentRunner::with_cli_path_and_timeout(
                path.to_string(),
                Duration::from_secs(10),
            ))
        },
        &config,
        tmp.path(),
    );
}

/// The json runner's spawned argv carries the overridden thinking level.
#[test]
fn json_runner_spawned_argv_carries_overridden_thinking_level() {
    let tmp = TempDir::new().unwrap();
    let config = resolved_config_with_override();
    assert_eq!(config.thinking_level, Some(ThinkingLevel::XHigh));

    assert_thinking_in_spawned_argv(
        |path| {
            Box::new(PiJsonAgentRunner::with_cli_path_and_timeout(
                path.to_string(),
                Duration::from_secs(10),
            ))
        },
        &config,
        tmp.path(),
    );
}
