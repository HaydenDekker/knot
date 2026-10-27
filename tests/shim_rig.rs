//! Shim rig integration tests (plan 100).
//!
//! A shim rig is a project-local rig whose workflow is served from an
//! external rig directory named by `external-rig` in
//! `rig/.workspace-agent-config.yaml`. These tests invoke the `knot`
//! binary to verify the startup contract:
//!
//! - missing / invalid external path → hard error, non-zero exit,
//!   clear `[KNOT][CONFIG]` diagnostic, nothing started
//! - valid external rig → looms discovered from it, runtime tree
//!   stays project-local (`tie-offs/rig/`)
//! - local looms/profiles in a shim rig → ignored, each listed in a
//!   startup warning

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

/// Resolve the path to the compiled `knot` binary.
fn binary_path() -> String {
    std::env::var("CARGO_BIN_EXE_knot").unwrap_or_else(|_| {
        format!(
            "{}/target/debug/knot",
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string())
        )
    })
}

/// Write a shim rig config pointing at `external`.
fn write_shim_config(rig_dir: &Path, external: &str) {
    fs::create_dir_all(rig_dir).unwrap();
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        format!("agent-adapter: pi-stdio\nexternal-rig: {external}\n"),
    )
    .unwrap();
}

/// Write a plain (non-shim) rig config.
fn write_plain_config(rig_dir: &Path) {
    fs::create_dir_all(rig_dir).unwrap();
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    )
    .unwrap();
}

/// Create a loom with one knot in `rig_dir`.
fn create_loom(rig_dir: &Path, name: &str) {
    let loom_dir = rig_dir.join(format!("{name}-loom"));
    fs::create_dir_all(&loom_dir).unwrap();
    fs::write(
        loom_dir.join(format!("{name}.md")),
        format!(
            "---\nname: {name}\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\n---\n\nTest knot {name}.\n"
        ),
    )
    .unwrap();
}

/// Create a `fast` profile in `rig_dir` (referenced by the knots).
fn create_fast_profile(rig_dir: &Path) {
    let profiles = rig_dir.join("profiles");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(
        profiles.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\n---\n\nReviewer.\n",
    )
    .unwrap();
}

/// Spawn `knot` in `cwd` and collect output until the process exits on
/// its own (hard-fail path) or `wait_ms` elapses (startup path).
struct KnotRun {
    child: Option<Child>,
}

impl KnotRun {
    fn start(cwd: &Path) -> Self {
        let child = Command::new(binary_path())
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("should spawn knot binary");
        Self { child: Some(child) }
    }

    /// Wait up to `ms` for the process to exit by itself.
    /// Returns Some(output) if it exited, None if still running.
    fn wait_exit(&mut self, ms: u64) -> Option<std::process::Output> {
        let deadline = std::time::Instant::now() + Duration::from_millis(ms);
        loop {
            let child = self.child.as_mut().expect("child present");
            if let Ok(Some(_)) = child.try_wait() {
                let child = self.child.take().unwrap();
                let out = child
                    .wait_with_output()
                    .expect("exited child should yield output");
                return Some(out);
            }
            if std::time::Instant::now() > deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Kill the process and return its output.
    fn kill_and_wait(mut self) -> std::process::Output {
        self.child
            .take()
            .map(|mut child| {
                let _ = child.kill();
                child.wait_with_output().expect("should wait for killed child")
            })
            .expect("child should exist")
    }
}

impl Drop for KnotRun {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Poll `tie-offs/rig/state.json` under `project` until `predicate`
/// matches or the deadline passes.
fn wait_for_state(
    project: &Path,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let state_path = project.join("tie-offs").join("rig").join("state.json");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(content) = fs::read_to_string(&state_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
            {
                if predicate(&value) {
                    return value;
                }
            }
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "timeout waiting for state.json predicate; last content: {:?}",
                fs::read_to_string(&state_path).ok()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn loom_ids(state: &serde_json::Value) -> Vec<String> {
    state["looms"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ── Hard-fail contract ───────────────────────────────────────────────────

#[test]
fn shim_with_missing_external_rig_refuses_to_start() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    write_shim_config(&project.join("rig"), "../gone-rig");

    let mut run = KnotRun::start(project);
    let out = run
        .wait_exit(5_000)
        .expect("knot must exit immediately on a missing external rig");

    assert!(
        !out.status.success(),
        "missing external rig must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[KNOT][CONFIG] ERROR"),
        "diagnostic must be a [KNOT][CONFIG] ERROR line, got: {stderr}"
    );
    assert!(
        stderr.contains("gone-rig") && stderr.contains("does not exist"),
        "diagnostic must name the missing path, got: {stderr}"
    );
    assert!(
        stderr.contains(".workspace-agent-config.yaml"),
        "diagnostic must name the declaring config file, got: {stderr}"
    );
    // Nothing started: no state.json, no looms.
    assert!(
        !project.join("tie-offs").join("rig").join("state.json").exists(),
        "nothing must start when the external rig is missing"
    );
}

#[test]
fn shim_with_file_external_rig_refuses_to_start() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    fs::write(project.join("not-a-dir"), "i am a file").unwrap();
    write_shim_config(&project.join("rig"), "not-a-dir");

    let mut run = KnotRun::start(project);
    let out = run
        .wait_exit(5_000)
        .expect("knot must exit immediately on a non-directory external rig");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is not a directory"),
        "diagnostic must say not-a-directory, got: {stderr}"
    );
}

#[test]
fn shim_with_empty_external_rig_refuses_to_start() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    // A bare `external-rig:` is YAML null — treated as absent (plain
    // rig). The error case is an explicit empty string.
    write_shim_config(&project.join("rig"), "\"\"");

    let mut run = KnotRun::start(project);
    let out = run
        .wait_exit(5_000)
        .expect("knot must exit immediately on an empty external-rig value");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("empty"),
        "diagnostic must flag the empty value, got: {stderr}"
    );
}

// ── Valid shim: workflow served from the external rig ────────────────────

#[test]
fn shim_discovers_looms_from_external_rig() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let external = project.join("shared/factory-rig");
    fs::create_dir_all(&external).unwrap();
    create_loom(&external, "ext");
    create_fast_profile(&external);
    write_shim_config(&project.join("rig"), "shared/factory-rig");

    let mut run = KnotRun::start(project);
    let state = wait_for_state(project, |s| loom_ids(s).contains(&"ext-loom".into()));

    assert!(
        run.wait_exit(300).is_none(),
        "service must stay running with a valid shim rig"
    );

    // Identity stays local: state's rig_path is the LOCAL rig dir,
    // not the external one.
    let rig_path = state["rig_path"].as_str().unwrap_or_default();
    assert!(
        rig_path.ends_with("/rig") && !rig_path.contains("factory-rig"),
        "rig_path must stay the local shim dir, got: {rig_path}"
    );

    // Provenance (plan 100 phase 4): state.json records the
    // resolved external rig path.
    let external_rig = state["external_rig"].as_str().unwrap_or_default();
    assert!(
        external_rig.ends_with("shared/factory-rig"),
        "state.json must record the external rig provenance, got: {state}"
    );

    // Runtime tree is project-local.
    assert!(project.join("tie-offs").join("rig").join("state.json").exists());
}

#[test]
fn shim_ignores_local_looms_and_profiles_with_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let external = project.join("shared/factory-rig");
    fs::create_dir_all(&external).unwrap();
    create_loom(&external, "ext");
    create_fast_profile(&external);

    // Local rig carries its own loom and profile — both must be
    // ignored (and listed) while the shim is active.
    let local_rig = project.join("rig");
    write_shim_config(&local_rig, "shared/factory-rig");
    create_loom(&local_rig, "local");
    create_fast_profile(&local_rig);

    let mut run = KnotRun::start(project);
    let state = wait_for_state(project, |s| loom_ids(s).contains(&"ext-loom".into()));

    assert!(
        !loom_ids(&state).contains(&"local-loom".into()),
        "local loom must NOT be registered in a shim rig, got: {:?}",
        loom_ids(&state)
    );

    let out = run.kill_and_wait();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("shim rig active")
            && stderr.contains("factory-rig"),
        "startup must log the active shim rig, got: {stderr}"
    );
    assert!(
        stderr.contains("ignores local loom 'local-loom'"),
        "startup must warn about the ignored local loom, got: {stderr}"
    );
    assert!(
        stderr.contains("ignores local profile 'fast.md'"),
        "startup must warn about the ignored local profile, got: {stderr}"
    );
}

#[test]
fn plain_rig_without_external_key_is_unaffected() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let local_rig = project.join("rig");
    write_plain_config(&local_rig);
    create_loom(&local_rig, "local");
    create_fast_profile(&local_rig);

    let mut run = KnotRun::start(project);
    let state =
        wait_for_state(project, |s| loom_ids(s).contains(&"local-loom".into()));
    assert!(loom_ids(&state).contains(&"local-loom".into()));

    let out = run.kill_and_wait();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("shim rig active"),
        "plain rig must not log shim lines, got: {stderr}"
    );
    // A plain rig's state.json carries no external_rig field.
    assert!(
        state.get("external_rig").is_none(),
        "plain rig state.json must not carry external_rig: {state}"
    );
}
