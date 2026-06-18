//! Integration tests for rig lifecycle.
//!
//! Verifies rig directory auto-creation, loom scanning on startup,
//! and state file content via filesystem reads.

#[path = "helpers.rs"]
mod helpers;

use std::fs;

use helpers::*;

/// Rig directory is auto-created on startup if it doesn't exist.
#[test]
fn rig_directory_auto_created() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    assert!(!rig_dir.exists());

    let handle = start_knot(rig_dir.clone());

    // State file proves the rig directory was created
    let state = wait_for_state_file(&rig_dir);
    assert_eq!(
        state.get("rig_path")
            .and_then(|v| v.as_str())
            .map(|s| s.ends_with("rig")),
        Some(true)
    );

    handle.abort();
}

/// Looms present at startup are scanned and registered.
#[test]
fn looms_scanned_on_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    let looms = state.get("looms").and_then(|v| v.as_array());
    assert!(looms.is_some());
    assert_eq!(looms.unwrap().len(), 1);

    handle.abort();
}

/// Empty rig (no looms) produces valid empty state.
#[test]
fn empty_rig_produces_valid_state() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    assert!(
        state.get("looms").and_then(|v| v.as_array()).unwrap().is_empty()
    );
    assert!(
        state.get("profiles")
            .and_then(|v| v.as_array())
            .unwrap()
            .is_empty()
    );
    assert!(state.get("rig_path").is_some());
    assert!(state.get("updated_at").is_some());

    handle.abort();
}

/// Profiles are loaded and written to state.json.
#[test]
fn profiles_loaded_into_state() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    create_fast_profile(&rig_dir);
    create_agent_profile(
        &rig_dir, "detailed", "anthropic", "claude-sonnet",
        "You are a detailed reviewer.",
    );

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    let profiles = state.get("profiles").and_then(|v| v.as_array());
    assert!(profiles.is_some());
    assert_eq!(profiles.unwrap().len(), 2);

    handle.abort();
}

/// State file has required schema fields.
#[test]
fn state_file_has_required_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    // Verify all required top-level fields
    assert!(state.get("rig_path").is_some(), "missing rig_path");
    assert!(state.get("looms").is_some(), "missing looms");
    assert!(state.get("profiles").is_some(), "missing profiles");
    assert!(state.get("updated_at").is_some(), "missing updated_at");

    // Verify types
    assert!(state.get("rig_path").unwrap().is_string());
    assert!(state.get("looms").unwrap().is_array());
    assert!(state.get("profiles").unwrap().is_array());
    assert!(state.get("updated_at").unwrap().is_string());

    handle.abort();
}
