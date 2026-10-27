//! Shim rig — model-registry separation tests (plan 100, phase 2).
//!
//! The shim rig keeps project-side configuration local: `models.yml`
//! and `.workspace-agent-config.yaml` always resolve from the **local**
//! rig directory. The external rig supplies the workflow (looms,
//! knots, profiles) — its own `models.yml` and config file are never
//! loaded. An external profile's `model-ref` alias therefore resolves
//! against the consuming project's registry.

#[path = "helpers.rs"]
mod helpers;

use std::fs;

use helpers::*;

/// External profile aliases resolve against the LOCAL models.yml;
/// the external rig's own models.yml is never read.
#[test]
fn external_profile_alias_resolves_against_local_models_yml() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let local_rig = project.join("rig");
    let external = project.join("shared/factory-rig");
    fs::create_dir_all(&external).unwrap();

    // ── Local shim rig: config points at the external rig; the
    //    local registry defines the `fast` alias with local values.
    fs::create_dir_all(&local_rig).unwrap();
    fs::write(
        local_rig.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\nexternal-rig: shared/factory-rig\n",
    )
    .unwrap();
    fs::write(
        local_rig.join("models.yml"),
        "models:\n  fast:\n    provider: localprov\n    model: localmodel\n",
    )
    .unwrap();

    // ── External rig: a profile referencing the `fast` alias, and
    //    its OWN models.yml with different values — which must NOT
    //    be loaded.
    let profiles = external.join("profiles");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(
        profiles.join("fast.md"),
        "---\nname: fast\nmodel-ref: fast\n---\n\nReviewer.\n",
    )
    .unwrap();
    fs::write(
        external.join("models.yml"),
        "models:\n  fast:\n    provider: otherprov\n    model: othermodel\n",
    )
    .unwrap();

    // A loom in the external rig so the service has a workflow.
    let loom = external.join("ext-loom");
    fs::create_dir_all(&loom).unwrap();
    fs::write(
        loom.join("ext.md"),
        "---\nname: ext\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\n---\n\nExt knot.\n",
    )
    .unwrap();

    let config = knot::AppConfig::with_rig_dir(local_rig.clone());
    let handle = start_knot_with_config(config);

    // The external loom is discovered into the project's state.
    wait_for_loom_in_state(&local_rig, "ext-loom", 1);

    let state = read_state_file(&local_rig).unwrap();
    let fast = state["profiles"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == "fast"))
        .unwrap_or_else(|| panic!("external profile 'fast' missing: {state}"));

    assert_eq!(
        fast["provider"].as_str(),
        Some("localprov"),
        "alias must resolve against the LOCAL models.yml"
    );
    assert_eq!(
        fast["model"].as_str(),
        Some("localmodel"),
        "alias must resolve against the LOCAL models.yml"
    );
    assert_ne!(
        fast["provider"].as_str(),
        Some("otherprov"),
        "the external rig's own models.yml must never be loaded"
    );

    handle.abort();
}

/// The external rig's own .workspace-agent-config.yaml is not loaded:
/// the local shim's `agent-adapter` governs. Verified by the local
/// config being pi-stdio while the external rig's config asks for
/// pi-json — the service starts and serves the external workflow with
/// the local adapter (no external-config diagnostics, local rig
/// identity in state).
#[test]
fn external_rig_config_file_is_not_loaded() {
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

    // The external rig carries its own config file — it must be
    // inert (not loaded, not merged).
    fs::write(
        external.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\ninactivity-timeout-seconds: 7\n",
    )
    .unwrap();

    let loom = external.join("ext-loom");
    fs::create_dir_all(&loom).unwrap();
    fs::write(
        loom.join("ext.md"),
        "---\nname: ext\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\n---\n\nExt knot.\n",
    )
    .unwrap();
    create_fast_profile(&external);

    let config = knot::AppConfig::with_rig_dir(local_rig.clone());
    let handle = start_knot_with_config(config);

    wait_for_loom_in_state(&local_rig, "ext-loom", 1);

    let state = read_state_file(&local_rig).unwrap();
    // Local identity retained.
    assert!(
        state["rig_path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("/rig"),
        "rig_path must stay the local shim dir: {state}"
    );

    handle.abort();
}
