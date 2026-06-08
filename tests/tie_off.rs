//! Tie-off lifecycle and section parsing tests.
//!
//! Verifies append-mode tie-off history and markdown section structure.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

/// Integration test: full tie-off lifecycle with append mode.
///
/// Note: file watchers may coalesce create+write into a single Modified event,
/// so we test the lifecycle as: first write (Modified) → second write (Modified)
/// → delete (Deleted), verifying append mode preserves history across events.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_tie_off_history() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("history-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) =
        make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Simple mock agent — always returns "processed"
    let mock_agent = create_mock_agent(&base_dir, "processed");

    let port = 31999;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await.expect("server should start listening");

    let strand_path = strand_dir.join("lifecycle-strand.md");
    let tie_off_path = tie_off_dir.join("lifecycle-strand.md.output");

    // Step 1: First write (triggers Modified event)
    fs::write(&strand_path, "initial content").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    assert!(
        tie_off_path.exists(),
        "tie-off should exist after first write (expected: {})",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    // Should have event metadata (Modified or Created depending on watcher)
    assert!(
        content.contains("## Event:")
            && content.contains("## Strand:")
            && content.contains("## Timestamp:"),
        "should have event metadata headers: {}", content
    );
    assert!(
        content.contains("processed"),
        "should have agent response: {}", content
    );

    // Step 2: Second write (triggers another Modified event)
    fs::write(&strand_path, "modified content").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    // Should have two sections now
    let delimiter_count = content.matches("---").count();
    assert!(
        delimiter_count >= 2,
        "should have at least 2 sections with delimiters, found {}: {}",
        delimiter_count, content
    );
    // Both sections should have event headers
    let event_count = content.matches("## Event:").count();
    assert!(
        event_count >= 2,
        "should have at least 2 event sections, found {}: {}",
        event_count, content
    );

    // Step 3: Delete strand (triggers Deleted event)
    fs::remove_file(&strand_path).unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("## Event: Deleted"),
        "should have Deleted section: {}", content
    );
    // Should have 3 sections now
    let delimiter_count = content.matches("---").count();
    assert!(
        delimiter_count >= 4,
        "should have 3 sections with delimiters, found {}: {}",
        delimiter_count, content
    );
    let event_count = content.matches("## Event:").count();
    assert!(
        event_count >= 3,
        "should have 3 event sections, found {}: {}",
        event_count, content
    );

    // Verify chronological order: Deleted should come last
    let first_event = content.find("## Event:").unwrap();
    let deleted_event = content.rfind("## Event: Deleted").unwrap();
    assert!(
        first_event < deleted_event,
        "Deleted should come after earlier events"
    );

    let _ = shutdown_tx.send(());
}

/// Integration test: parse tie-off markdown sections and verify structure.
///
/// Creates a strand, modifies it, and verifies that the tie-off file
/// contains properly formatted sections with event type, strand path,
/// and timestamp metadata.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tie_off_sections_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("sections-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) =
        make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let mock_agent = create_mock_agent(&base_dir, "processed");

    let port = 32000;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await.expect("server should start listening");

    let strand_path = strand_dir.join("sections-strand.md");
    let tie_off_path = tie_off_dir.join("sections-strand.md.output");

    // Create then modify
    fs::write(&strand_path, "content v1").unwrap();
    std::thread::sleep(Duration::from_millis(800));
    fs::write(&strand_path, "content v2").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Split into sections using --- as delimiter.
    // Structure: [header] --- [content] --- [header] --- [content] ...
    let sections: Vec<&str> = content
        .split("---")
        .filter(|s| !s.trim().is_empty())
        .collect();

    assert!(
        sections.len() >= 2,
        "should have at least 2 sections, found {}: {}",
        sections.len(), content
    );

    // Collect all header sections (those containing ## Event:)
    let header_sections: Vec<&str> = sections
        .iter()
        .filter(|s| s.contains("## Event:"))
        .copied()
        .collect();

    assert!(
        header_sections.len() >= 2,
        "should have at least 2 header sections, found {}: {}",
        header_sections.len(), content
    );

    // Verify each header section has complete metadata
    for (i, section) in header_sections.iter().enumerate() {
        assert!(
            section.contains("## Event:"),
            "header section {} should have event type: {}",
            i, section
        );
        assert!(
            section.contains("## Strand:"),
            "header section {} should have strand path: {}",
            i, section
        );
        assert!(
            section.contains("## Timestamp:"),
            "header section {} should have timestamp: {}",
            i, section
        );

        // Verify timestamp format (ISO 8601)
        if let Some(ts_start) = section.find("## Timestamp:") {
            let ts_line = section[ts_start..]
                .lines()
                .next()
                .unwrap_or("");
            let ts_value = ts_line
                .trim()
                .strip_prefix("## Timestamp:")
                .unwrap_or("")
                .trim();
            assert!(
                ts_value.contains('T') && ts_value.ends_with('Z'),
                "timestamp should be ISO 8601 format, got: {}",
                ts_value
            );
        }

        // Verify strand path is present
        if let Some(strand_start) = section.find("## Strand:") {
            let strand_line = section[strand_start..]
                .lines()
                .next()
                .unwrap_or("");
            let strand_value = strand_line
                .trim()
                .strip_prefix("## Strand:")
                .unwrap_or("")
                .trim();
            assert!(
                !strand_value.is_empty(),
                "strand path should not be empty"
            );
            assert!(
                strand_value.contains("sections-strand.md"),
                "strand path should reference the strand file: {}",
                strand_value
            );
        }
    }

    // Verify markdown structure: sections separated by --- (horizontal rule)
    assert!(
        content.contains("\n---\n"),
        "tie-off should have --- delimiter between sections: {}",
        content
    );

    let _ = shutdown_tx.send(());
}
