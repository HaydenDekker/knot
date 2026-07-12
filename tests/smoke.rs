//! Smoke tests — full composition with mock agent via `cli_path` injection.
//!
//! These tests spin up the complete Knot runtime (file watching, debounce,
//! state writer, subprocess agent) but use a deterministic mock agent
//! binary. They verify the composition wiring end-to-end without any
//! process-global PATH or env-var manipulation.

#[path = "helpers.rs"]
mod helpers;

use std::fs;

use helpers::*;

/// Full composition smoke test using the pi-stdio adapter.
///
/// Spins up the real Knot runtime with a mock `pi` binary that echoes
/// a fixed response. Verifies that the agent response appears in the
/// tie-off file and that knot status reaches "completed" in state.json.
#[test]
fn composition_smoke_stdio() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path().to_path_buf();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    // ── Mock agent: reads stdin, echoes response, exits 0 ──
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    fs::write(
        &pi_path,
        "#!/usr/bin/env bash\n\
         # Mock pi for smoke test - consumes stdin, echoes response\n\
         cat > /dev/null\n\
         echo \"review complete\"\n\
         exit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    // ── Rig configuration: pi-stdio adapter ──
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    )
    .unwrap();

    // ── Loom + knot definition ──
    let loom_dir = rig_dir.join("review-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    fs::write(
        loom_dir.join("review.md"),
        "---\n\
         name: review\n\
         agent-profile-ref: fast\n\
         strand-dir: \"./strands\"\n\
         git-versioned: false\n\
         ---\n\
         \n\
         Review knot.\n\
         \n",
    )
    .unwrap();

    // ── Agent profile ──
    create_fast_profile(&rig_dir);

    // ── Start Knot with cli_path pointing to mock agent ──
    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path.clone());
    let handle = start_knot_with_config(config);

    // Wait for loom discovery
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // ── Create strand file (triggers processing via notify watch) ──
    let strands_dir = project_root.join("strands");
    fs::create_dir_all(&strands_dir).unwrap();
    fs::write(strands_dir.join("feature.md"), "new feature").unwrap();

    // Wait for knot to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // ── Verify tie-off file ──
    let tie_off_file = rig_dir.join("tie-offs/review-loom/tie-off-review.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist at {}",
        tie_off_file.display()
    );

    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("review complete"),
        "tie-off should contain mock agent response. Got:\n{}",
        content
    );

    handle.abort();
}

/// Full composition smoke test using the pi-json adapter.
///
/// Same as `composition_smoke_stdio` but with `agent-adapter: pi-json`.
/// The mock agent outputs JSON-L (session + agent_end) instead of plain
/// text stdout.
#[test]
fn composition_smoke_json() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path().to_path_buf();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    // ── Mock agent: outputs JSON-L with session ID + agent response ──
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    fs::write(
        &pi_path,
        "#!/usr/bin/env bash\n\
         # Mock pi (json mode) - consumes stdin, outputs JSON-L\n\
         cat > /dev/null\n\
         echo '{\"type\":\"session\",\"id\":\"smoke-sess-123\"}'\n\
         echo '{\"type\":\"agent_end\",\"usage\":{\"input\":10,\"output\":5,\"cache_read\":0,\"cache_write\":0,\"total\":15},\"messages\":[{\"role\":\"assistant\",\"stopReason\":\"stop\",\"content\":[{\"type\":\"text\",\"text\":\"review complete\"}]}]}'\n\
         exit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    // ── Rig configuration: pi-json adapter ──
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    )
    .unwrap();

    // ── Loom + knot definition ──
    let loom_dir = rig_dir.join("review-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    fs::write(
        loom_dir.join("review.md"),
        "---\n\
         name: review\n\
         agent-profile-ref: fast\n\
         strand-dir: \"./strands\"\n\
         git-versioned: false\n\
         ---\n\
         \n\
         Review knot.\n\
         \n",
    )
    .unwrap();

    // ── Agent profile ──
    create_fast_profile(&rig_dir);

    // ── Start Knot with cli_path pointing to mock agent ──
    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path.clone());
    let handle = start_knot_with_config(config);

    // Wait for loom discovery
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // ── Create strand file (triggers processing via notify watch) ──
    let strands_dir = project_root.join("strands");
    fs::create_dir_all(&strands_dir).unwrap();
    fs::write(strands_dir.join("feature.md"), "new feature").unwrap();

    // Wait for knot to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // ── Verify tie-off file ──
    let tie_off_file = rig_dir.join("tie-offs/review-loom/tie-off-review.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist at {}",
        tie_off_file.display()
    );

    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("review complete"),
        "tie-off should contain parsed JSON response. Got:\n{}",
        content
    );

    handle.abort();
}
