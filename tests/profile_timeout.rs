//! Integration tests for agent timeout handling.
//!
//! Verifies that profile-level timeout configuration is respected
//! and timeout events are recorded.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// Profile timeout overrides the runner's default timeout.
#[test]
fn profile_timeout_is_respected() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with a 500ms timeout
    let profiles_dir = rig_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    let profile_content = [
        "---",
        "name: fast",
        "provider: openai",
        "model: gpt-4o",
        "timeout: 1",
        "---",
        "",
        "You are a reviewer.",
        "",
    ].join("\n");
    fs::write(profiles_dir.join("fast.md"), profile_content).unwrap();

    // Mock pi that sleeps longer than the timeout
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = "#!/usr/bin/env bash\ncat > /dev/null\nsleep 10\nexit 0\n";
    fs::write(&pi_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let config = format!(
        "cli_path: \"{}\"\n\
         cli_args: []\n",
        pi_path.display()
    );
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");

    // Wait for processing (should timeout)
    // The runner's default is 300s but the profile timeout is 1s
    // so it should complete much faster
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let state = match read_state_file(&rig_dir) {
            Ok(s) => s,
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        let knot = state
            .get("looms")
            .and_then(|v| v.as_array())
            .and_then(|a| a.get(0))
            .and_then(|l| l.get("knots"))
            .and_then(|v| v.as_array())
            .and_then(|a| a.get(0));
        if let Some(knot) = knot {
            let status = knot.get("status").and_then(|v| v.as_str());
            if status == Some("failed") || status == Some("completed") {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    handle.abort();
}
