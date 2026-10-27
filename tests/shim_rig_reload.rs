//! Shim rig — live hot-reload over the external rig (plan 100, phase 3).
//!
//! The rig watch follows the **effective** (external) rig directory, so
//! edits to the external rig — new looms, knot changes — are picked up
//! live by the running project, with the same debounce/reload
//! semantics as local-rig edits.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::time::Duration;

use helpers::*;

/// A loom added to the external rig after startup is discovered live.
#[test]
fn loom_added_to_external_rig_is_discovered_live() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let local_rig = project.join("rig");
    let external = project.join("shared/factory-rig");
    fs::create_dir_all(&external).unwrap();

    fs::create_dir_all(&local_rig).unwrap();
    fs::write(
        local_rig.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\nexternal-rig: shared/factory-rig\n",
    )
    .unwrap();
    create_fast_profile(&external);

    // Initial loom so the service is up and watching.
    let loom = external.join("first-loom");
    fs::create_dir_all(&loom).unwrap();
    fs::write(
        loom.join("first.md"),
        "---\nname: first\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\n---\n\nFirst knot.\n",
    )
    .unwrap();

    let config = knot::AppConfig::with_rig_dir(local_rig.clone());
    let handle = start_knot_with_config(config);
    wait_for_loom_in_state(&local_rig, "first-loom", 1);

    // ── Add a second loom to the EXTERNAL rig after startup ──
    let loom2 = external.join("second-loom");
    fs::create_dir_all(&loom2).unwrap();
    fs::write(
        loom2.join("second.md"),
        "---\nname: second\nagent-profile-ref: fast\nstrand-dir: \"./strands2\"\n---\n\nSecond knot.\n",
    )
    .unwrap();

    // The new loom must appear in the project's state without a
    // restart — the watch followed the effective rig.
    wait_for_loom_in_state(&local_rig, "second-loom", 1);

    handle.abort();
}

/// A knot file edited in the external rig is re-read live: the knot's
/// strand dir change is picked up, so a strand dropped in the NEW
/// directory triggers the knot.
#[test]
fn knot_edited_in_external_rig_reloads_live() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let local_rig = project.join("rig");
    let external = project.join("shared/factory-rig");
    fs::create_dir_all(&external).unwrap();

    fs::create_dir_all(&local_rig).unwrap();
    fs::write(
        local_rig.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\nexternal-rig: shared/factory-rig\n",
    )
    .unwrap();
    create_fast_profile(&external);

    let loom = external.join("edit-loom");
    fs::create_dir_all(&loom).unwrap();
    let knot_file = loom.join("edit.md");
    fs::write(
        &knot_file,
        "---\nname: edit\nagent-profile-ref: fast\nstrand-dir: \"./strands-a\"\n---\n\nEdit knot.\n",
    )
    .unwrap();

    let config = knot::AppConfig::with_rig_dir(local_rig.clone())
        .with_cli_path(mock_agent_path(project));
    let handle = start_knot_with_config(config);
    wait_for_loom_in_state(&local_rig, "edit-loom", 1);

    // ── Edit the external knot: repoint strand-dir to strands-b ──
    fs::write(
        &knot_file,
        "---\nname: edit\nagent-profile-ref: fast\nstrand-dir: \"./strands-b\"\n---\n\nEdit knot repointed.\n",
    )
    .unwrap();

    // Let the config pipeline process the KnotModified and register
    // the new strand watch before dropping a strand into it — the
    // creation event must arrive after the watch is up.
    std::thread::sleep(Duration::from_secs(2));

    // Drop a strand into the NEW dir — it must be picked up by the
    // reloaded watch.
    let strands_b = project.join("strands-b");
    fs::create_dir_all(&strands_b).unwrap();
    fs::write(strands_b.join("task.md"), "# new strand\n").unwrap();

    // The knot processes it: status reaches completed for the new
    // strand path.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state = read_state_file(&local_rig).unwrap();
        let done = state["looms"]
            .as_array()
            .map(|ls| {
                ls.iter().any(|l| {
                    l["knots"].as_array().map(|ks| {
                        ks.iter().any(|k| {
                            k["status"] == "completed"
                                && k["last_strand_path"]
                                    .as_str()
                                    .map(|s| s.contains("strands-b"))
                                    .unwrap_or(false)
                        })
                    }).unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if done {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "strand in the repointed dir was never processed; state: {}",
                serde_json::to_string_pretty(&state).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    handle.abort();
}

/// Mock agent binary (echoes a fixed response).
fn mock_agent_path(project: &std::path::Path) -> std::path::PathBuf {
    let bin = project.join("mockbin");
    fs::create_dir_all(&bin).unwrap();
    let pi = bin.join("pi");
    fs::write(
        &pi,
        "#!/usr/bin/env bash\ncat > /dev/null\necho \"done\"\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    }
    pi
}
