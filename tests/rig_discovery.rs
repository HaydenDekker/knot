//! Unit tests for rig discovery (domain layer).
//!
//! Tests `domain::rig_discovery::discover_rigs()` directly — no CLI,
//! no `AppConfig`, no server.

use std::fs;
use std::path::Path;

use knot::domain::rig_discovery::{discover_rigs, RigDiscovery};

fn make_rig_dir(parent: &Path, name: &str) {
    fs::create_dir_all(parent.join(name)).unwrap();
}

fn make_non_rig_dir(parent: &Path, name: &str) {
    fs::create_dir_all(parent.join(name)).unwrap();
}

// ── Zero matches ───────────────────────────────────────────────────────────

#[test]
fn discover_rigs_zero_matches_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let result = discover_rigs(tmp.path(), None);
    assert!(matches!(result, RigDiscovery::None));
}

// ── Single match ───────────────────────────────────────────────────────────

#[test]
fn discover_rigs_single_match_returns_single() {
    let tmp = tempfile::tempdir().unwrap();
    make_rig_dir(tmp.path(), "dev-rig");
    let result = discover_rigs(tmp.path(), None);
    match result {
        RigDiscovery::Single(path) => {
            assert_eq!(path.file_name().unwrap(), "dev-rig");
        }
        other => panic!("expected Single, got {other:?}"),
    }
}

// ── Multiple matches ──────────────────────────────────────────────────────

#[test]
fn discover_rigs_two_matches_returns_multiple() {
    let tmp = tempfile::tempdir().unwrap();
    make_rig_dir(tmp.path(), "dev-rig");
    make_rig_dir(tmp.path(), "review-rig");
    let result = discover_rigs(tmp.path(), None);
    match result {
        RigDiscovery::Multiple(paths) => {
            assert_eq!(paths.len(), 2);
            let names: Vec<&str> = paths
                .iter()
                .map(|p| p.file_name().unwrap().to_str().unwrap())
                .collect();
            assert!(names.contains(&"dev-rig"));
            assert!(names.contains(&"review-rig"));
        }
        other => panic!("expected Multiple, got {other:?}"),
    }
}

#[test]
fn discover_rigs_three_matches_returns_multiple() {
    let tmp = tempfile::tempdir().unwrap();
    make_rig_dir(tmp.path(), "dev-rig");
    make_rig_dir(tmp.path(), "review-rig");
    make_rig_dir(tmp.path(), "prod-rig");
    let result = discover_rigs(tmp.path(), None);
    match result {
        RigDiscovery::Multiple(paths) => {
            assert_eq!(paths.len(), 3);
        }
        other => panic!("expected Multiple, got {other:?}"),
    }
}

// ── Explicit name ─────────────────────────────────────────────────────────

#[test]
fn discover_rigs_explicit_name_returns_named() {
    let tmp = tempfile::tempdir().unwrap();
    make_rig_dir(tmp.path(), "dev-rig");
    make_rig_dir(tmp.path(), "review-rig");
    let result = discover_rigs(tmp.path(), Some("dev-rig"));
    match result {
        RigDiscovery::Named(path) => {
            assert_eq!(path.file_name().unwrap(), "dev-rig");
        }
        other => panic!("expected Named, got {other:?}"),
    }
}

// ── Non-rig directories ignored ───────────────────────────────────────────

#[test]
fn discover_rigs_ignores_non_rig_directories() {
    let tmp = tempfile::tempdir().unwrap();
    make_non_rig_dir(tmp.path(), "src");
    make_non_rig_dir(tmp.path(), "rig");
    make_non_rig_dir(tmp.path(), "planning-loom");
    make_non_rig_dir(tmp.path(), "node_modules");
    let result = discover_rigs(tmp.path(), None);
    assert!(matches!(result, RigDiscovery::None));
}

// ── Mixed rig and non-rig directories ─────────────────────────────────────

#[test]
fn discover_rigs_mixed_directories_single_rig() {
    let tmp = tempfile::tempdir().unwrap();
    make_rig_dir(tmp.path(), "dev-rig");
    make_non_rig_dir(tmp.path(), "src");
    make_non_rig_dir(tmp.path(), "planning-loom");
    make_non_rig_dir(tmp.path(), "rig");
    let result = discover_rigs(tmp.path(), None);
    match result {
        RigDiscovery::Single(path) => {
            assert_eq!(path.file_name().unwrap(), "dev-rig");
        }
        other => panic!("expected Single, got {other:?}"),
    }
}
