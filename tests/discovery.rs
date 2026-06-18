//! Integration tests for loom discovery.
//!
//! Verifies that Knot discovers looms from the rig directory via
//! file-watching and writes them to `rig/state.json`.

#[path = "helpers.rs"]
mod helpers;

use std::fs;

use knot::AppConfig;
use helpers::*;

/// Knot discovers looms present at startup and writes them to state.json.
#[test]
fn discovers_looms_at_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    // Create a loom with a knot before starting
    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let config = AppConfig::with_rig_dir(rig_dir.clone());
    let (ctx, strand_rx, config_rx) = knot::build_app_context(&config);

    // Run startup discovery
    let looms = knot::run_startup(&ctx, &rig_dir).unwrap();
    assert_eq!(looms.len(), 1);
    assert_eq!(looms[0].id.0, "review-loom");
    assert_eq!(looms[0].knots.len(), 1);
    assert_eq!(looms[0].knots[0].id.0, "review");

    // Drop receivers to avoid warnings
    drop(strand_rx);
    drop(config_rx);
}

/// Knot ignores directories that don't end in `-loom`.
#[test]
fn ignores_non_loom_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    // Create non-loom directories
    fs::create_dir_all(rig_dir.join("src")).unwrap();
    fs::create_dir_all(rig_dir.join("data")).unwrap();
    fs::create_dir_all(rig_dir.join("profiles")).unwrap();
    create_fast_profile(&rig_dir);

    let config = AppConfig::with_rig_dir(rig_dir.clone());
    let (ctx, strand_rx, config_rx) = knot::build_app_context(&config);

    // Run startup discovery
    let looms = knot::run_startup(&ctx, &rig_dir).unwrap();
    assert!(looms.is_empty(), "should find no looms");

    drop(strand_rx);
    drop(config_rx);
}

/// Knot discovers multiple looms in a single rig.
#[test]
fn discovers_multiple_looms() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    // Create two looms
    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");

    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");

    let config = AppConfig::with_rig_dir(rig_dir.clone());
    let (ctx, strand_rx, config_rx) = knot::build_app_context(&config);

    let looms = knot::run_startup(&ctx, &rig_dir).unwrap();
    assert_eq!(looms.len(), 2);

    let ids: Vec<_> = looms.iter().map(|l| l.id.0.as_str()).collect();
    assert!(ids.contains(&"review-loom"));
    assert!(ids.contains(&"planning-loom"));

    drop(strand_rx);
    drop(config_rx);
}

/// Knot writes discovered looms to state.json via the state writer task.
#[test]
fn writes_discovered_looms_to_state_file() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Start Knot in background
    let handle = start_knot(rig_dir.clone());

    // Wait for state.json to be written
    let state = wait_for_state_file(&rig_dir);

    // Verify state contains the discovered loom
    let looms = state.get("looms").and_then(|v| v.as_array());
    assert!(looms.is_some(), "state should contain looms array");
    let looms = looms.unwrap();
    assert_eq!(looms.len(), 1);

    let loom = &looms[0];
    assert_eq!(
        loom.get("id").and_then(|v| v.as_str()),
        Some("review-loom")
    );
    assert_eq!(
        loom.get("knots")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len()),
        Some(1)
    );

    // Verify profile is also in state
    let profiles = state.get("profiles").and_then(|v| v.as_array());
    assert!(profiles.is_some());
    assert_eq!(profiles.unwrap().len(), 1);

    handle.abort();
}
