//! Acceptance tests for model aliases (069 phase 4).
//!
//! Real `FileSystemModelRegistry`, `FileSystemAgentProfileRepository`,
//! `FileSystemLoomLog`, and `FileSystemTieOffSink` against a `tempfile`
//! rig tree, with `MockAgentRunner` standing in for the CLI.
//!
//! Covers:
//! 1. Full flow — `models.yml` + `model-ref` profile → tie-off written,
//!    runner received the resolved `--model`.
//! 2. Live swap — rewriting `models.yml` between two strand runs; the
//!    second run uses the new model, no restart.
//! 3. Unknown alias — knot fails; error visible in the loom-log and in
//!    state `last_error` (via a real `WriteState` build).
//! 4. Regression — legacy direct-spec profile with no `models.yml`
//!    still runs.
//! 5. `run_startup` auto-creates `rig/models.yml`; a second run does not
//!    overwrite it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use knot::adapters::outbound::{
    ContentInspectorChecker, FileSystemAgentProfileRepository,
    FileSystemLoomLog, FileSystemModelRegistry, FileSystemRigLog,
    FileSystemTieOffSink,
};
use knot::application::ports::{
    AgentProfileRepository, AgentRunner, LoomLogPort, ModelRegistryPort,
    PortError, StateWriterPort, StrandEventQueue,
};
use knot::application::store::LoomStore;
use knot::application::usecases::test_fixtures::{
    build_knot_with_profile, build_loom, MockAgentRunner, MockEventDispatcher,
    MockGitVersioningPort,
};
use knot::application::usecases::{ProcessStrand, WriteState};
use knot::domain::entities::{Loom, LoomId, RigState, StrandPath};
use knot::domain::events::{LoomEvent, StrandEvent};
use knot::domain::knot_file::derive_runtime_root;
use knot::domain::value_objects::RigAgentConfig;
use knot::{AppConfig, build_app_context, run_startup};
use tempfile::TempDir;

// ── Fixtures ────────────────────────────────────────────────────────────

/// Write a profile file with the given frontmatter fields and body
/// (profile prompt) to `{rig_dir}/profiles/{name}.md`.
fn write_profile(rig_dir: &Path, name: &str, frontmatter: &str, body: &str) {
    let dir = rig_dir.join("profiles");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{name}.md")),
        format!("---\n{frontmatter}---\n{body}\n"),
    )
    .unwrap();
}

/// Write `rig/models.yml` with the given content.
fn write_models_yml(rig_dir: &Path, content: &str) {
    fs::create_dir_all(rig_dir).unwrap();
    fs::write(rig_dir.join("models.yml"), content).unwrap();
}

/// Alias profile: `model-ref: {alias}`.
fn alias_profile_md(name: &str, alias: &str) -> String {
    format!("name: {name}\nmodel-ref: {alias}\n")
}

/// Legacy direct-spec profile.
fn direct_profile_md(name: &str, provider: &str, model: &str) -> String {
    format!("name: {name}\nprovider: {provider}\nmodel: {model}\n")
}

/// A pipeline wired with the real filesystem adapters (registry,
/// profiles, loom-log, tie-off sink, rig-log) and a mock runner.
struct Pipeline {
    strand: ProcessStrand,
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
    profile_repo: Arc<dyn AgentProfileRepository>,
    model_registry: Arc<dyn ModelRegistryPort>,
    rig_dir: PathBuf,
}

/// Build a `ProcessStrand` against a real `tempfile` rig tree.
fn build_pipeline(
    rig_dir: PathBuf,
    looms: Vec<Loom>,
    runner: Arc<MockAgentRunner>,
) -> Pipeline {
    let store = LoomStore::new();
    for loom in looms {
        store.register(loom);
    }

    let runtime_root = derive_runtime_root(&rig_dir);
    let log_port: Arc<dyn LoomLogPort> =
        Arc::new(FileSystemLoomLog::new(rig_dir.clone()));
    let profile_repo: Arc<dyn AgentProfileRepository> = Arc::new(
        FileSystemAgentProfileRepository::new(rig_dir.join("profiles")),
    );
    let model_registry: Arc<dyn ModelRegistryPort> =
        Arc::new(FileSystemModelRegistry::new(rig_dir.clone()));

    let strand = ProcessStrand::new(
        store.clone(),
        log_port.clone(),
        runner as Arc<dyn AgentRunner>,
        Arc::new(FileSystemTieOffSink::new(rig_dir.clone())),
        RigAgentConfig::default_config(),
        rig_dir.clone(),
        profile_repo.clone(),
        model_registry.clone(),
        Arc::new(FileSystemRigLog::new(runtime_root)),
        Arc::new(MockGitVersioningPort::default()),
        Arc::new(ContentInspectorChecker),
        Arc::new(MockEventDispatcher::default()),
        None,
    );

    Pipeline {
        strand,
        store,
        log_port,
        profile_repo,
        model_registry,
        rig_dir,
    }
}

/// A `StateWriterPort` collector for asserting on built state.
#[derive(Default)]
struct StateCollector {
    states: Arc<Mutex<Vec<RigState>>>,
}

impl StateWriterPort for StateCollector {
    fn write_state(&self, state: &RigState) -> Result<(), PortError> {
        self.states.lock().unwrap().push(state.clone());
        Ok(())
    }
}

fn created_event(strand_path: PathBuf) -> StrandEvent {
    StrandEvent::Created {
        loom_id: LoomId("test-loom".to_string()),
        knot_id: knot::domain::entities::KnotId("k1".to_string()),
        strand_path: StrandPath(strand_path),
    }
}

// ── 1. Full flow ────────────────────────────────────────────────────────

/// `models.yml` + `model-ref` profile → the runner receives the
/// resolved `--model` and the tie-off is written to the runtime root.
#[test]
fn full_flow_alias_profile_resolves_model_and_writes_tieoff() {
    let tmp = TempDir::new().unwrap();
    let rig_dir = tmp.path().join("rig");
    write_models_yml(
        &rig_dir,
        "models:\n  fast:\n    provider: openai\n    model: gpt-4o-mini\n",
    );
    write_profile(&rig_dir, "fast", &alias_profile_md("fast", "fast"), "You are fast.");

    let runner = Arc::new(MockAgentRunner::default());
    let loom = build_loom(
        "test-loom",
        vec![build_knot_with_profile("k1", "fast")],
    );
    let pipeline = build_pipeline(rig_dir.clone(), vec![loom], runner.clone());

    let dir = tmp.path().to_path_buf();
    let strand_path = dir.join("strand.md");
    fs::write(&strand_path, "test content").unwrap();

    let result = pipeline.strand.execute(created_event(strand_path));
    assert!(result.is_ok(), "processing should succeed");

    // Runner got the resolved model from the registry.
    let ctx = runner.get_captured_ctx().expect("runner should have executed");
    assert_eq!(ctx.agent_config.provider, "openai");
    assert_eq!(ctx.agent_config.model, "gpt-4o-mini");
    let args = ctx.agent_config.build_cli_args();
    let model_index = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model flag missing");
    assert_eq!(args[model_index + 1], "gpt-4o-mini");

    // Tie-off written under the runtime root.
    let tie_off_path = derive_runtime_root(&rig_dir)
        .join("test-loom")
        .join("tie-off-k1.md");
    assert!(
        tie_off_path.exists(),
        "tie-off should be written at {}",
        tie_off_path.display()
    );
    let content = fs::read_to_string(&tie_off_path).unwrap();
    assert!(
        content.contains("mock"),
        "tie-off should contain agent output: {content}"
    );
}

// ── 2. Live swap ────────────────────────────────────────────────────────

/// Rewriting `models.yml` between two strand runs swaps the model
/// behind the alias — the second run uses the new model, no restart.
#[test]
fn live_swap_rewrites_models_yml_between_runs() {
    let tmp = TempDir::new().unwrap();
    let rig_dir = tmp.path().join("rig");
    write_models_yml(
        &rig_dir,
        "models:\n  fast:\n    provider: openai\n    model: gpt-4o\n",
    );
    write_profile(&rig_dir, "fast", &alias_profile_md("fast", "fast"), "You are fast.");

    let runner = Arc::new(MockAgentRunner::default());
    let loom = build_loom(
        "test-loom",
        vec![build_knot_with_profile("k1", "fast")],
    );
    let pipeline = build_pipeline(rig_dir.clone(), vec![loom], runner.clone());

    // Run 1 — original model.
    let strand1 = tmp.path().join("strand-1.md");
    fs::write(&strand1, "first").unwrap();
    assert!(pipeline.strand.execute(created_event(strand1)).is_ok());
    let ctx1 = runner.get_captured_ctx().expect("run 1 should execute");
    assert_eq!(ctx1.agent_config.model, "gpt-4o");

    // Swap the registry file (no restart).
    write_models_yml(
        &rig_dir,
        "models:\n  fast:\n    provider: anthropic\n    model: claude-sonnet-4-20250514\n",
    );

    // Run 2 — new model must be picked up.
    let strand2 = tmp.path().join("strand-2.md");
    fs::write(&strand2, "second").unwrap();
    assert!(pipeline.strand.execute(created_event(strand2)).is_ok());
    let ctx2 = runner.get_captured_ctx().expect("run 2 should execute");
    assert_eq!(ctx2.agent_config.provider, "anthropic");
    assert_eq!(ctx2.agent_config.model, "claude-sonnet-4-20250514");

    let all = runner.get_captured_contexts();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].agent_config.model, "gpt-4o");
    assert_eq!(all[1].agent_config.model, "claude-sonnet-4-20250514");
}

// ── 3. Unknown alias failure path ───────────────────────────────────────

/// Unknown alias → the knot run fails; the error is visible in the
/// loom-log and in state `last_error`.
#[test]
fn unknown_alias_fails_knot_and_surfaces_in_loom_log_and_state() {
    let tmp = TempDir::new().unwrap();
    let rig_dir = tmp.path().join("rig");
    write_models_yml(
        &rig_dir,
        "models:\n  fast:\n    provider: openai\n    model: gpt-4o\n",
    );
    // Profile references an alias that does not exist in the registry.
    write_profile(&rig_dir, "fast", &alias_profile_md("fast", "ghost"), "You are fast.");

    let runner = Arc::new(MockAgentRunner::default());
    let loom = build_loom(
        "test-loom",
        vec![build_knot_with_profile("k1", "fast")],
    );
    let pipeline = build_pipeline(rig_dir.clone(), vec![loom], runner.clone());

    let strand_path = tmp.path().join("strand.md");
    fs::write(&strand_path, "test content").unwrap();

    let result = pipeline.strand.execute(created_event(strand_path));
    match result {
        Err(PortError::ModelRefNotFound(alias)) => assert_eq!(alias, "ghost"),
        other => panic!(
            "expected Err(ModelRefNotFound), got {:?}",
            other.is_ok()
        ),
    }

    // Loom-log carries the failure.
    let events = pipeline
        .log_port
        .read_all(&LoomId("test-loom".to_string()))
        .unwrap();
    let failed = events
        .iter()
        .find_map(|e| match e {
            LoomEvent::KnotFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .expect("loom-log should contain KnotFailed");
    assert!(failed.contains("ghost"), "error should name the alias: {failed}");
    assert!(
        failed.contains("rig/models.yml"),
        "error should point at the registry file: {failed}"
    );

    // State: the knot is failed with the error in `last_error`.
    let collector = Arc::new(StateCollector::default());
    let write_state = WriteState::new(
        pipeline.store.clone(),
        pipeline.log_port.clone(),
        pipeline.profile_repo.clone(),
        pipeline.model_registry.clone(),
        collector.clone() as Arc<dyn StateWriterPort>,
        pipeline.rig_dir.clone(),
        Arc::new(Mutex::new(None::<Arc<dyn StrandEventQueue>>)),
    );
    write_state.execute().unwrap();

    let states = collector.states.lock().unwrap();
    assert_eq!(states.len(), 1, "state should have been written once");
    let state = &states[0];
    let loom = state
        .looms
        .iter()
        .find(|l| l.id == "test-loom")
        .expect("state should contain the loom");
    let knot = loom.knots.iter().find(|k| k.id == "k1").expect("knot in state");
    assert_eq!(knot.status, "failed");
    assert!(
        knot.last_error.as_deref().unwrap_or_default().contains("ghost"),
        "last_error should name the alias: {:?}",
        knot.last_error
    );

    // The unresolvable alias is visible in the profile entry as nulls.
    let profile = state
        .profiles
        .iter()
        .find(|p| p.name == "fast")
        .expect("state should contain the profile");
    assert_eq!(profile.model_ref.as_deref(), Some("ghost"));
    assert_eq!(profile.provider, None);
    assert_eq!(profile.model, None);
}

// ── 4. Legacy direct-spec regression ────────────────────────────────────

/// A legacy direct-spec profile with no `models.yml` on disk still runs
/// and uses its own provider/model values.
#[test]
fn legacy_direct_profile_runs_without_models_yml() {
    let tmp = TempDir::new().unwrap();
    let rig_dir = tmp.path().join("rig");
    // No models.yml — the file is absent entirely.
    write_profile(
        &rig_dir,
        "fast",
        &direct_profile_md("fast", "openai", "gpt-4o"),
        "You are fast.",
    );
    assert!(!rig_dir.join("models.yml").exists());

    let runner = Arc::new(MockAgentRunner::default());
    let loom = build_loom(
        "test-loom",
        vec![build_knot_with_profile("k1", "fast")],
    );
    let pipeline = build_pipeline(rig_dir, vec![loom], runner.clone());

    let strand_path = tmp.path().join("strand.md");
    fs::write(&strand_path, "test content").unwrap();

    let result = pipeline.strand.execute(created_event(strand_path));
    assert!(result.is_ok(), "direct-spec profile should run without a registry");

    let ctx = runner.get_captured_ctx().expect("runner should have executed");
    assert_eq!(ctx.agent_config.provider, "openai");
    assert_eq!(ctx.agent_config.model, "gpt-4o");
}

// ── 5. Startup auto-create ──────────────────────────────────────────────

/// `run_startup` creates `rig/models.yml` (commented template) when
/// missing; a second run never overwrites an existing file.
#[test]
fn run_startup_creates_models_yml_and_never_overwrites() {
    let tmp = TempDir::new().unwrap();
    let rig_dir = tmp.path().join("rig");
    assert!(!rig_dir.exists());

    let config = AppConfig::with_rig_dir(rig_dir.clone());
    let (ctx, _strand_rx, _config_rx) = build_app_context(&config);
    run_startup(&ctx, &rig_dir).unwrap();

    // File created with the template.
    let models_path = rig_dir.join("models.yml");
    assert!(models_path.exists(), "models.yml should be auto-created");
    let original = fs::read_to_string(&models_path).unwrap();
    assert!(
        original.contains("Rig-level model registry"),
        "auto-created file should be the commented template"
    );

    // Second run does NOT overwrite user content.
    let custom = "models:\n  default:\n    provider: anthropic\n    model: claude-sonnet-4-20250514\n";
    fs::write(&models_path, custom).unwrap();
    run_startup(&ctx, &rig_dir).unwrap();
    let after = fs::read_to_string(&models_path).unwrap();
    assert_eq!(after, custom, "existing models.yml must not be overwritten");
}
