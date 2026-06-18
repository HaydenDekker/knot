//! Integration tests for skill file validation and API contracts.
//!
//! Verifies that skill files exist with correct paths and that
//! state file schema matches what skills expect.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::path::Path;

use helpers::*;

// ── State File Schema Tests ────────────────────────────────────────────

/// State file has `rig_path` field that skills use.
#[test]
fn state_file_has_rig_path() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    assert!(state.get("rig_path").is_some());
    assert!(state.get("rig_path").unwrap().is_string());

    handle.abort();
}

/// State file `looms` array contains loom objects with `id` and `knots`.
#[test]
fn state_file_looms_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(looms.len(), 1);

    let loom = &looms[0];
    assert!(loom.get("id").is_some());
    assert!(loom.get("knots").is_some());
    assert!(loom.get("knots").unwrap().is_array());

    handle.abort();
}

/// State file `profiles` array contains profile objects.
#[test]
fn state_file_profiles_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    let profiles = state.get("profiles").and_then(|v| v.as_array()).unwrap();
    assert_eq!(profiles.len(), 1);

    let profile = &profiles[0];
    assert!(profile.get("name").is_some());
    assert!(profile.get("provider").is_some());
    assert!(profile.get("model").is_some());

    handle.abort();
}

/// State file knot objects have `status` field.
#[test]
fn state_file_knot_has_status() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    let knot = state
        .get("looms")
        .and_then(|v| v.as_array())
        .unwrap().get(0).unwrap()
        .get("knots")
        .and_then(|v| v.as_array())
        .unwrap().get(0).unwrap();

    assert!(knot.get("id").is_some());
    assert!(knot.get("status").is_some());
    assert_eq!(
        knot.get("status").and_then(|v| v.as_str()),
        Some("idle")
    );

    handle.abort();
}

/// State file `updated_at` is an ISO 8601 timestamp.
#[test]
fn state_file_updated_at_is_timestamp() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);

    let ts = state.get("updated_at").and_then(|v| v.as_str()).unwrap();
    assert!(ts.starts_with("20"), "should be a date starting with 20");
    assert!(ts.contains("T"), "should contain T separator");
    assert!(ts.ends_with("Z"), "should end with Z (UTC)");

    handle.abort();
}

// ── File Path Convention Tests ─────────────────────────────────────────

/// Loom directory naming convention: ends in `-loom`.
#[test]
fn loom_directory_naming_convention() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    assert!(
        loom_dir.file_name().unwrap() == "review-loom",
        "loom dir should end with -loom"
    );
}

/// Knot definition files are `.md` files in the loom directory.
#[test]
fn knot_file_naming_convention() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    let knot_file = loom_dir.join("review.md");
    assert!(knot_file.exists());
    assert_eq!(knot_file.extension().unwrap(), "md");
}

/// Profile files are `.md` files in `profiles/` directory.
#[test]
fn profile_file_naming_convention() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let profile = rig_dir.join("profiles/fast.md");
    assert!(profile.exists());
    assert_eq!(profile.extension().unwrap(), "md");
}

/// Tie-off files are at `tie-offs/{loom-id}/{knot-name}/{knot-id}-tie-off.md`.
#[test]
fn tie_off_path_convention() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    let tie_off = rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    assert!(tie_off.exists());

    handle.abort();
}

/// Loom-log is at `tie-offs/{loom-id}/.loom-log`.
#[test]
fn loom_log_path_convention() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    let log_path = rig_dir.join("tie-offs/review-loom/.loom-log");
    assert!(log_path.exists());

    handle.abort();
}
