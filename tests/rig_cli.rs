//! Integration tests for the Knot CLI — rig switching and sharing.
//!
//! Invokes the `knot` binary via `std::process::Command` to verify:
//! - Multiple rigs found (no args) → non-zero exit with rig names
//! - Single rig found (no args) → server starts, process stays alive
//! - Named rig → server starts with rig directory created
//! - Share command → zip contains looms + profiles, excludes tie-offs

use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use zip::ZipArchive;

/// Resolve the path to the compiled `knot` binary.
///
/// `cargo test` sets `CARGO_BIN_EXE_<name>` for each binary target.
/// Falls back to the default target directory path.
fn binary_path() -> String {
    std::env::var("CARGO_BIN_EXE_knot").unwrap_or_else(|_| {
        format!(
            "{}/target/debug/knot",
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string())
        )
    })
}

/// Helper: write a knot definition file inside a loom directory.
fn write_knot(loom_dir: &std::path::Path, name: &str) {
    let content = format!(
        "---\nname: {}\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\n---\n\nTest knot: {}.\n",
        name, name
    );
    fs::write(loom_dir.join(format!("{}.md", name)), content).unwrap();
}

/// Helper: write a fast agent profile into a rig's profiles directory.
fn write_fast_profile(rig_dir: &std::path::Path) {
    let profiles_dir = rig_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\n---\n\nYou are a reviewer.\n",
    )
    .unwrap();
}

/// Helper: create a loom directory inside a rig.
fn create_loom(rig_dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let loom_path = rig_dir.join(format!("{}-loom", name));
    fs::create_dir_all(&loom_path).unwrap();
    loom_path
}

/// Helper: spawn the knot binary and ensure it terminates on drop.
struct KnotProcess {
    child: Option<Child>,
}

impl KnotProcess {
    fn spawn(current_dir: &std::path::Path, args: &[&str]) -> Self {
        let child = Command::new(binary_path())
            .current_dir(current_dir)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("should spawn knot binary");

        Self { child: Some(child) }
    }

    /// Check if the process is still running.
    fn is_alive(&mut self) -> bool {
        self.child
            .as_mut()
            .and_then(|c| c.try_wait().ok())
            .map(|s| s.is_none())
            .unwrap_or(false)
    }

    /// Kill the process and return its output.
    fn kill_and_wait(mut self) -> std::process::Output {
        self.child
            .take()
            .map(|mut child| {
                let _ = child.kill();
                child.wait_with_output().expect("should wait for child")
            })
            .expect("child should exist")
    }
}

impl Drop for KnotProcess {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ── Multiple rigs, no args → error ─────────────────────────────────────────

#[test]
fn cli_multiple_rigs_no_args_exits_with_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Create two rig directories
    fs::create_dir_all(cwd.join("dev-rig")).unwrap();
    fs::create_dir_all(cwd.join("review-rig")).unwrap();

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        !output.status.success(),
        "expected non-zero exit, got status: {}",
        output.status
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dev-rig"),
        "stderr should mention dev-rig, got: {}",
        stderr
    );
    assert!(
        stderr.contains("review-rig"),
        "stderr should mention review-rig, got: {}",
        stderr
    );
}

// ── Single rig, no args → process stays alive ─────────────────────────────

#[test]
fn cli_single_rig_no_args_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Create a single rig directory
    let rig = cwd.join("my-rig");
    fs::create_dir_all(&rig).unwrap();
    write_fast_profile(&rig);

    let mut knot = KnotProcess::spawn(cwd, &[]);

    // Wait for the process to start
    thread::sleep(Duration::from_secs(2));

    // Check if the process is still running (didn't crash)
    let still_alive = knot.is_alive();

    let output = knot.kill_and_wait();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        still_alive,
        "server process should be running.\n\
         still_alive={}, stderr: {}",
        still_alive,
        stderr
    );
}

// ── Share command → zip with looms + profiles, no tie-offs ─────────────────

#[test]
fn cli_share_creates_zip_with_looms_and_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Create rig with looms, profiles, and tie-offs
    let rig = cwd.join("dev-rig");
    fs::create_dir_all(&rig).unwrap();
    write_fast_profile(&rig);

    // Create a loom with a knot file
    let loom = create_loom(&rig, "definition");
    write_knot(&loom, "review");

    // Create the project-side runtime tree (tie-offs live outside the
    // rig now, so they can never enter the zip)
    let tie_offs = cwd.join("tie-offs").join("dev-rig").join("definition-loom");
    fs::create_dir_all(&tie_offs).unwrap();
    fs::write(tie_offs.join("tie-off-review.md"), "output").unwrap();

    // Run share command
    let output = Command::new(binary_path())
        .current_dir(cwd)
        .args(["share", "dev-rig"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        output.status.success(),
        "share command should succeed, got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify zip file was created
    let zip_path = cwd.join("dev-rig.zip");
    assert!(
        zip_path.exists(),
        "zip file should exist at {}",
        zip_path.display()
    );

    // Open and inspect zip contents
    let file = fs::File::open(&zip_path).expect("should open zip file");
    let mut archive = ZipArchive::new(file).expect("should open zip archive");

    // Collect all entry names
    let entries: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();

    // Should contain the loom knot file
    let has_knot = entries
        .iter()
        .any(|e| e == "dev-rig/definition-loom/review.md");
    assert!(
        has_knot,
        "zip should contain loom knot file.\nEntries: {:?}",
        entries
    );

    // Should contain the profile
    let has_profile = entries.iter().any(|e| e == "dev-rig/profiles/fast.md");
    assert!(
        has_profile,
        "zip should contain profile.\nEntries: {:?}",
        entries
    );

    // Should NOT contain tie-offs (derived state)
    let has_tie_offs = entries.iter().any(|e| e.contains("tie-off"));
    assert!(
        !has_tie_offs,
        "zip should NOT contain tie-offs.\nEntries: {:?}",
        entries
    );

    // Should NOT contain .rig-log
    let has_rig_log = entries.iter().any(|e| e.contains("rig-log"));
    assert!(
        !has_rig_log,
        "zip should NOT contain rig-log.\nEntries: {:?}",
        entries
    );
}

// ── Share command on non-existent rig → error ──────────────────────────────

#[test]
fn cli_share_nonexistent_rig_exits_with_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .args(["share", "no-such-rig"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        !output.status.success(),
        "expected non-zero exit for missing rig"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no-such-rig"),
        "stderr should mention the missing rig name, got: {}",
        stderr
    );
}

// ── Named rig that doesn't exist → auto-created on startup ────────────────

#[test]
fn cli_named_rig_does_not_exist_creates_it() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Don't create any rig — let knot create it
    let new_rig = cwd.join("new-rig");
    assert!(
        !new_rig.exists(),
        "rig should not exist before test"
    );

    let mut knot = KnotProcess::spawn(cwd, &["new-rig"]);

    // Wait for the process to start
    thread::sleep(Duration::from_secs(2));

    let still_alive = knot.is_alive();

    // Rig directory should have been created by run_startup
    assert!(
        new_rig.exists(),
        "rig directory should have been created at {}",
        new_rig.display()
    );

    let output = knot.kill_and_wait();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        still_alive,
        "server process should be running.\n\
         still_alive={}, stderr: {}",
        still_alive,
        stderr
    );
}

// ── Share: rig has no looms → still produces valid zip ────────────────────

#[test]
fn cli_share_rig_with_no_looms_produces_valid_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Create an empty rig (no looms, no profiles)
    let rig = cwd.join("empty-rig");
    fs::create_dir_all(&rig).unwrap();

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .args(["share", "empty-rig"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        output.status.success(),
        "share should succeed even with no looms, got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify zip file was created
    let zip_path = cwd.join("empty-rig.zip");
    assert!(
        zip_path.exists(),
        "zip file should exist at {}",
        zip_path.display()
    );

    // Verify zip is valid (openable, even if empty)
    let file = fs::File::open(&zip_path).expect("should open zip file");
    let archive = ZipArchive::new(file).expect("should open zip archive");
    assert_eq!(
        archive.len(),
        0,
        "zip should be empty when rig has no looms"
    );
}

// ── Share: rig has profiles but no looms → zip has profiles only ──────────

#[test]
fn cli_share_rig_profiles_only_produces_valid_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Create rig with profiles but no looms
    let rig = cwd.join("profiles-only-rig");
    fs::create_dir_all(&rig).unwrap();
    write_fast_profile(&rig);

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .args(["share", "profiles-only-rig"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        output.status.success(),
        "share should succeed, got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let zip_path = cwd.join("profiles-only-rig.zip");
    assert!(
        zip_path.exists(),
        "zip file should exist at {}",
        zip_path.display()
    );

    let file = fs::File::open(&zip_path).expect("should open zip file");
    let mut archive = ZipArchive::new(file).expect("should open zip archive");

    let entries: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();

    assert!(
        entries.iter().any(|e| e.contains("profiles/fast.md")),
        "zip should contain profile. Entries: {:?}",
        entries
    );
}

// ── Unknown flag → error with usage hint ──────────────────────────────────

#[test]
fn cli_unknown_flag_exits_with_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .arg("--unknown-flag")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown flag"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown") || stderr.contains("unsupported")
            || stderr.contains("unrecognized"),
        "stderr should mention unknown/unsupported flag, got: {}",
        stderr
    );
}

// ── Share command without rig name → error ─────────────────────────────────

#[test]
fn cli_share_without_rig_name_exits_with_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .arg("share")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");

    assert!(
        !output.status.success(),
        "expected non-zero exit when share has no rig name"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("share requires") || stderr.contains("rig name"),
        "stderr should explain that a rig name is required, got: {}",
        stderr
    );
}

// ── Legacy layout → auto-migration at startup ──────────────────────────────

/// A rig on the legacy layout (runtime artifacts inside the rig dir) is
/// auto-migrated to the project-level runtime root at startup —
/// including before the state writer and event queue create the runtime
/// root themselves (regression: those tasks used to spawn first and
/// turn `rig/tie-offs/` and `rig/events/` into false conflicts).
///
/// Plan 083: the migrated `.loom-log` / `.rig-log` files are inert —
/// they survive the move untouched and are never read, cleared, or
/// appended to. The discovery evidence now lives on stderr (`[KNOT]
/// [EVENT] LoomStarted …`), and the state writer replaces the moved
/// `state.json` with the current snapshot.
#[test]
fn cli_startup_migrates_legacy_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // Legacy layout: runtime artifacts inside the rig directory
    let rig = cwd.join("rig");
    write_fast_profile(&rig);
    let loom = create_loom(&rig, "review");
    write_knot(&loom, "k");
    let legacy_tieoffs = rig.join("tie-offs").join("review-loom");
    fs::create_dir_all(&legacy_tieoffs).unwrap();
    fs::write(legacy_tieoffs.join("tie-off-k.md"), "tie-off").unwrap();
    fs::write(legacy_tieoffs.join(".loom-log"), "legacy-line\n").unwrap();
    fs::write(rig.join("state.json"), "{}\n").unwrap();
    fs::write(rig.join(".rig-log"), "legacy\n").unwrap();
    fs::create_dir_all(rig.join("events")).unwrap();

    let knot = KnotProcess::spawn(cwd, &[]);

    // Give startup (migration, discovery, first state write) time
    thread::sleep(Duration::from_secs(2));

    let output = knot.kill_and_wait();
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Tie-off subtree moved to the runtime root
    let runtime_root = cwd.join("tie-offs").join("rig");
    assert!(
        runtime_root.join("review-loom").join("tie-off-k.md").exists(),
        "legacy tie-offs should be migrated.\nstderr: {stderr}"
    );
    // Moved loom-log at the new path: inert (plan 083) — its legacy
    // content survives untouched and nothing is appended to it.
    let loom_log = fs::read_to_string(
        runtime_root.join("review-loom").join(".loom-log"),
    )
    .unwrap_or_default();
    assert!(
        loom_log.lines().any(|l| l.contains("legacy-line")),
        "moved loom-log must be untouched (inert).\nstderr: {stderr}"
    );
    // Discovery is proven by the `[EVENT]` line on stderr (the log
    // files are no longer the event record).
    assert!(
        stderr.contains("[KNOT][EVENT] LoomStarted loom=review-loom"),
        "discovery must emit the LoomStarted `[EVENT]` line\n\nstderr: {stderr}"
    );
    // The events dir moved — not lost to a conflict with the queue's
    // own directory creation
    assert!(
        runtime_root.join("events").is_dir(),
        "legacy events dir should be migrated.\nstderr: {stderr}"
    );
    assert!(
        !rig.join("events").exists(),
        "rig/events must not remain after migration.\nstderr: {stderr}"
    );
    // state.json at the runtime root (the moved legacy `{}` replaced
    // by the state writer's first write); the moved `.rig-log` is
    // inert.
    let state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(runtime_root.join("state.json")).unwrap(),
    )
    .unwrap_or(serde_json::Value::Null);
    assert!(
        state.get("looms").is_some(),
        "state.json should be the current snapshot at the runtime root.\nstderr: {stderr}"
    );
    assert!(
        runtime_root.join(".rig-log").exists(),
        ".rig-log should be at the runtime root (inert).\nstderr: {stderr}"
    );
    // Rig left source-only
    assert!(!rig.join("tie-offs").exists(), "rig left source-only");
    assert!(!rig.join("state.json").exists());
    assert!(!rig.join(".rig-log").exists());
    // Migration notice logged
    assert!(
        stderr.contains("migrated legacy layout"),
        "stderr should carry the migration notice.\nstderr: {stderr}"
    );
}
