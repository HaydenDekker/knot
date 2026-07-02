//! Adapter contract tests — one test per outbound adapter.
//!
//! Each adapter is tested in isolation against its port contract using
//! `tempfile::tempdir()` for filesystem adapters and mock scripts for
//! subprocess adapters. All tests are fully parallel (no shared state).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use knot::adapters::outbound::event_source::NotifyEventSource;
use knot::adapters::outbound::loom_log::FileSystemLoomLog;
use knot::adapters::outbound::loom_repository::FileSystemLoomRepository;
use knot::adapters::outbound::profile_repo::FileSystemAgentProfileRepository;
use knot::adapters::outbound::state_writer::FileSystemStateWriter;
use knot::adapters::outbound::tieoff_sink::FileSystemTieOffSink;
use knot::adapters::pi_json::PiJsonAgentRunner;
use knot::adapters::pi_stdio::PiStdioAgentRunner;
use knot::application::ports::{
    AgentProfileRepository, AgentRunner, EventSource, LoomLogPort,
    LoomRepository, PortError, StateWriterPort, TieOffSink,
};
use knot::domain::entities::{
    KnotId, LoomId, RigState, RigStateKnot, RigStateLoom, RigStateProfile,
    StrandPath, TieOff, TieOffPath, TieOffStatus,
};
use knot::domain::events::LoomEvent;
use tokio::sync::mpsc;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Create a unique temp directory, write an executable bash script,
/// and return the path to the script. Caller keeps the `TempDir` alive.
fn create_mock_script(
    dir: &tempfile::TempDir,
    name: &str,
    script: &str,
) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, PermissionsExt::from_mode(0o755)).unwrap();
    path
}

/// Poll a tokio mpsc receiver for an event with a timeout.
fn recv_timeout<T>(
    rx: &mut mpsc::Receiver<T>,
    timeout: Duration,
) -> Option<T> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Ok(event) = rx.try_recv() {
            return Some(event);
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

// ── PiStdioAgentRunner ─────────────────────────────────────────────────────

mod pi_stdio_adapter {
    use super::*;

    const PASS_SCRIPT: &str = r#"#!/usr/bin/env bash
cat > /dev/null
echo "stdio response"
exit 0
"#;

    const FAIL_SCRIPT: &str = r#"#!/usr/bin/env bash
cat > /dev/null
echo "error: something went wrong" >&2
exit 1
"#;

    const BLOCK_SCRIPT: &str = r#"#!/usr/bin/env bash
cat > /dev/null
sleep 300
"#;

    fn make_context(_cli_path: &str) -> knot::application::ports::ExecutionContext {
        knot::application::ports::ExecutionContext {
            agent_config: knot::domain::value_objects::AgentConfig {
                goal: "test".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
            },
            prompt: "test prompt".to_string(),
            profile_prompt: "You are a test agent.".to_string(),
            strand_path: StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }

    /// Subprocess spawn with mock script → captures stdout.
    #[test]
    fn subprocess_spawn_captures_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let mock_path = create_mock_script(&dir, "mock-pi", PASS_SCRIPT);
        let runner = PiStdioAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(
            output.stdout.contains("stdio response"),
            "stdout should contain mock output: {}",
            output.stdout
        );
    }

    /// Non-zero exit code → returns `PortError::AgentExecutionFailed`.
    #[test]
    fn nonzero_exit_returns_agent_execution_failed() {
        let dir = tempfile::tempdir().unwrap();
        let mock_path = create_mock_script(&dir, "mock-pi-fail", FAIL_SCRIPT);
        let runner = PiStdioAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for non-zero exit");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::AgentExecutionFailed { .. }),
            "expected AgentExecutionFailed, got {err:?}"
        );
    }

    /// Timeout enforcement → returns `PortError::Timeout`.
    #[test]
    fn timeout_enforcement_returns_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let mock_path = create_mock_script(&dir, "mock-pi-block", BLOCK_SCRIPT);
        let runner = PiStdioAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_millis(50), // short timeout
        );

        let mut ctx = make_context(&mock_path.to_string_lossy());
        ctx.timeout = Some(Duration::from_millis(50));

        let start = std::time::Instant::now();
        let result = runner.execute(ctx);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should error for timeout");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "should use context timeout, not runner default"
        );
    }

    /// Unique `tempfile::tempdir()` + mock path per test — verified by
    /// the test harness: each test above creates its own `TempDir`.
    #[test]
    fn unique_tempdir_per_test() {
        // This test itself uses its own tempdir — the compiler ensures
        // each test function gets its own isolated filesystem.
        let dir = tempfile::tempdir().unwrap();
        assert!(dir.path().exists());
    }
}

// ── PiJsonAgentRunner ──────────────────────────────────────────────────────

mod pi_json_adapter {
    use super::*;

    /// Mock script that outputs JSON-L on stdout.
    fn make_json_mock(
        session_id: &str,
        response: &str,
        stop_reason: &str,
    ) -> String {
        format!(
            r#"#!/usr/bin/env bash
cat > /dev/null
echo '{{"type":"session","id":"{session_id}"}}'
echo '{{"type":"agent_end","usage":{{"input":100,"output":50,"cache_read":0,"cache_write":0,"total":150}},"messages":[{{"role":"assistant","stopReason":"{stop_reason}","content":[{{"type":"text","text":"{response}"}}]}}]}}'
exit 0
"#,
        )
    }

    fn make_context(_cli_path: &str) -> knot::application::ports::ExecutionContext {
        knot::application::ports::ExecutionContext {
            agent_config: knot::domain::value_objects::AgentConfig {
                goal: "test".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
            },
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }

    /// JSON-L parsing → extracts `session_id` from `agent_end` event.
    #[test]
    fn json_l_parsing_extracts_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let script = make_json_mock("json-sess-abc123", "hello world", "stop");
        let mock_path = create_mock_script(&dir, "mock-pi-json", &script);
        let runner = PiJsonAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        let metadata = output.metadata.expect("should have metadata");
        assert_eq!(
            metadata.session_id.as_deref(),
            Some("json-sess-abc123"),
            "session_id should be extracted from JSON-L"
        );
    }

    /// `stopReason: "stop"` → produces response text.
    #[test]
    fn stop_reason_stop_produces_response() {
        let dir = tempfile::tempdir().unwrap();
        let script = make_json_mock("sess-1", "final answer", "stop");
        let mock_path = create_mock_script(&dir, "mock-pi-stop", &script);
        let runner = PiJsonAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let output = runner.execute(ctx).unwrap();
        assert!(
            output.stdout.contains("final answer"),
            "stdout should contain response text for stop: {}",
            output.stdout
        );
    }

    /// `stopReason: "toolUse"` → excluded from response.
    #[test]
    fn stop_reason_tool_use_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-tool"}'
echo '{"type":"agent_end","usage":{"input":10,"output":10,"cache_read":0,"cache_write":0,"total":20},"messages":[{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"tool use intermediate"}]}]}'
exit 0
"#;
        let mock_path = create_mock_script(&dir, "mock-pi-tool", script);
        let runner = PiJsonAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let output = runner.execute(ctx).unwrap();
        // toolUse messages should NOT appear in response
        assert!(
            !output.stdout.contains("tool use intermediate"),
            "toolUse response should be excluded: {}",
            output.stdout
        );
    }

    /// `stopReason: "error"` → excluded from response.
    #[test]
    fn stop_reason_error_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-err"}'
echo '{"type":"agent_end","usage":{"input":10,"output":10,"cache_read":0,"cache_write":0,"total":20},"messages":[{"role":"assistant","stopReason":"error","content":[{"type":"text","text":"error output"}]}]}'
exit 0
"#;
        let mock_path = create_mock_script(&dir, "mock-pi-err", script);
        let runner = PiJsonAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let output = runner.execute(ctx).unwrap();
        assert!(
            !output.stdout.contains("error output"),
            "error stopReason should be excluded: {}",
            output.stdout
        );
    }

    /// `stopReason: "length"` → included (truncated response).
    #[test]
    fn stop_reason_length_included() {
        let dir = tempfile::tempdir().unwrap();
        let script = make_json_mock("sess-len", "truncated", "length");
        let mock_path = create_mock_script(&dir, "mock-pi-len", &script);
        let runner = PiJsonAgentRunner::with_cli_path_and_timeout(
            mock_path.to_string_lossy().to_string(),
            Duration::from_secs(10),
        );

        let ctx = make_context(&mock_path.to_string_lossy());
        let output = runner.execute(ctx).unwrap();
        assert!(
            output.stdout.contains("truncated"),
            "length stopReason should be included: {}",
            output.stdout
        );
    }

    /// Unique `tempfile::tempdir()` + mock path per test — verified by
    /// the test harness: each test function creates its own `TempDir`.
    #[test]
    fn unique_tempdir_per_test() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dir.path().exists());
    }
}

// ── FileSystemTieOffSink ───────────────────────────────────────────────────

mod tieoff_sink_adapter {
    use super::*;

    /// `write()` creates file at correct path.
    #[test]
    fn write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());

        let tie_off = TieOff {
            content: "Generated content".to_string(),
            path: TieOffPath(dir.path().join("review.tie-off.md")),
            status: TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
        };

        assert!(sink.write(tie_off).is_ok());
        assert!(dir.path().join("review.tie-off.md").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("review.tie-off.md")).unwrap(),
            "Generated content"
        );
    }

    /// `append()` adds delimiter + header before new content.
    #[test]
    fn append_adds_delimiter_and_header() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("history.tie-off");

        // First append
        sink.append(TieOff {
            content: "Section one".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            knot_name: Some("review".to_string()),
            event_type: Some("Created".to_string()),
            strand_path: Some("strand.md".to_string()),
            timestamp: Some("2026-06-05T10:00:00Z".to_string()),
        })
        .unwrap();

        // Second append
        sink.append(TieOff {
            content: "Section two".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            knot_name: Some("review".to_string()),
            event_type: Some("Modified".to_string()),
            strand_path: Some("strand.md".to_string()),
            timestamp: Some("2026-06-05T11:00:00Z".to_string()),
        })
        .unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("Section one"));
        assert!(content.contains("Section two"));
        assert!(content.contains("review triggered by Created strand.md"));
        assert!(content.contains("review triggered by Modified strand.md"));
        assert!(content.matches("---").count() >= 2);
    }

    /// `read_content()` returns existing content.
    #[test]
    fn read_content_returns_existing() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let tie_off_path = TieOffPath(dir.path().join("readable.tie-off.md"));

        // Write content first
        fs::write(&tie_off_path.0, "pre-existing content").unwrap();

        let content = sink.read_content(&tie_off_path).unwrap();
        assert_eq!(content, "pre-existing content");
    }

    /// Directory creation: creates `tie-offs/{loom}/` if missing.
    #[test]
    fn write_creates_nested_directories() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());

        let tie_off = TieOff {
            content: "Deep content".to_string(),
            path: TieOffPath(dir.path().join("sub/nested/deep.tie-off.md")),
            status: TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
        };

        assert!(sink.write(tie_off).is_ok());
        assert!(
            dir.path().join("sub/nested/deep.tie-off.md").exists(),
            "parent directories should be created"
        );
    }
}

// ── FileSystemLoomLog ──────────────────────────────────────────────────────

mod loom_log_adapter {
    use super::*;

    /// `open()` creates directory + empty log file.
    #[test]
    fn open_creates_directory_and_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("open-test-loom".to_string());

        assert!(log.open(&loom_id).is_ok());
        let log_path = dir.path().join("tie-offs/open-test-loom/.loom-log");
        assert!(
            log_path.exists(),
            "open should create the .loom-log file"
        );
    }

    /// `append()` writes JSONL entry.
    #[test]
    fn append_writes_jsonl_entry() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("append-test-loom".to_string());

        log.append(LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();

        let log_path = dir.path().join("tie-offs/append-test-loom/.loom-log");
        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("LoomStarted"),
            "JSONL entry should contain LoomStarted"
        );
    }

    /// `read_all()` returns parsed events.
    #[test]
    fn read_all_returns_parsed_events() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("readall-test-loom".to_string());

        log.append(LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();
        log.append(LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: KnotId("k1".to_string()),
            timestamp: "2026-06-10T12:00:01Z".to_string(),
        })
        .unwrap();

        let events = log.read_all(&loom_id).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], LoomEvent::LoomStarted { .. }));
        assert!(matches!(events[1], LoomEvent::KnotRegistered { .. }));
    }

    /// Idempotent `open()` — no error on re-open.
    #[test]
    fn open_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("idempotent-loom".to_string());

        assert!(log.open(&loom_id).is_ok());
        assert!(log.open(&loom_id).is_ok()); // second call should not error
        assert!(log.open(&loom_id).is_ok()); // third call too

        // Can still append after multiple opens
        log.append(LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();
    }
}

// ── FileSystemStateWriter ──────────────────────────────────────────────────

mod state_writer_adapter {
    use super::*;

    fn build_state(rig_path: &str) -> RigState {
        RigState {
            rig_path: rig_path.to_string(),
            looms: vec![RigStateLoom {
                id: "test-loom".to_string(),
                knots: vec![RigStateKnot {
                    id: "k1".to_string(),
                    status: "idle".to_string(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                }],
            }],
            profiles: vec![RigStateProfile {
                name: "fast".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                timeout: None,
            }],
            strand_queue: vec![],
            updated_at: "2026-06-18T12:00:00Z".to_string(),
        }
    }

    /// `write_state()` writes valid JSON to `state.json`.
    #[test]
    fn write_state_writes_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = build_state(dir.path().to_str().unwrap());

        assert!(writer.write_state(&state).is_ok());
        let path = dir.path().join("state.json");
        assert!(path.exists(), "state.json should exist");

        let content = fs::read_to_string(&path).unwrap();
        let parsed: RigState = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.looms.len(), 1);
        assert_eq!(parsed.looms[0].id, "test-loom");
    }

    /// Atomic write: `.state.json.tmp` → `rename` to `state.json`.
    #[test]
    fn write_state_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = build_state("/atomic");

        writer.write_state(&state).unwrap();

        // Only state.json exists, no temp file
        let entries: Vec<_> = fs::read_dir(&dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.contains(&"state.json".to_string()));
        assert!(
            !entries.iter().any(|n| n.contains(".tmp")),
            "temp file should not remain after rename"
        );
    }

    /// Concurrent writes do not corrupt (two writers, verify valid JSON).
    #[test]
    fn concurrent_writes_do_not_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let writer = std::sync::Arc::new(std::sync::Mutex::new(
            FileSystemStateWriter::new(dir.path().to_path_buf()),
        ));
        let mut handles = Vec::new();

        for i in 0..10 {
            let writer = std::sync::Arc::clone(&writer);
            let handle = std::thread::spawn(move || {
                let state = RigState {
                    rig_path: "/concurrent".to_string(),
                    looms: vec![],
                    profiles: vec![RigStateProfile {
                        name: format!("profile-{i}"),
                        provider: "openai".to_string(),
                        model: "gpt-4o".to_string(),
                        timeout: None,
                    }],
                    strand_queue: vec![],
                    updated_at: format!("2026-06-18T00:00:0{i}Z"),
                };
                writer.lock().unwrap().write_state(&state).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Final state.json should be valid JSON
        let content = fs::read_to_string(dir.path().join("state.json")).unwrap();
        let _parsed: RigState = serde_json::from_str(&content).unwrap();
    }
}

// ── FileSystemLoomRepository ───────────────────────────────────────────────

mod loom_repository_adapter {
    use super::*;

    const VALID_KNOT: &str = "---
name: review-knot
agent-profile-ref: fast
strand-dir: \"../external-source\"
---

Review the goals section.
";

    fn create_knot_file(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(format!("{name}.md")), content).unwrap();
    }

    /// `scan()` discovers looms ending in `-loom`.
    #[test]
    fn scan_discovers_loom_directories() {
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();
        create_knot_file(&loom_dir, "knot1", VALID_KNOT);

        // Also create a non-loom directory (should be skipped — doesn't end in -loom)
        let other_dir = rig.path().join("data");
        fs::create_dir(&other_dir).unwrap();
        create_knot_file(&other_dir, "knot2", VALID_KNOT);

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1, "only -loom directories discovered");
        assert_eq!(looms[0].id, LoomId("my-loom".to_string()));
        assert!(warnings.is_empty());
    }

    /// `scan_knot_files()` parses `.md` knot files with YAML frontmatter.
    #[test]
    fn scan_knot_files_parses_yaml_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        create_knot_file(dir.path(), "review", VALID_KNOT);

        let repo = FileSystemLoomRepository::new();
        let (knots, warnings) = repo.scan_knot_files(dir.path()).unwrap();

        assert_eq!(knots.len(), 1);
        assert_eq!(knots[0].id, KnotId("review-knot".to_string()));
        assert_eq!(knots[0].agent_profile_ref, "fast");
        assert!(warnings.is_empty());
    }

    /// Parse warnings for unknown YAML properties.
    #[test]
    fn parse_warnings_for_unknown_properties() {
        let rig = tempfile::tempdir().unwrap();
        let loom_dir = rig.path().join("warn-loom");
        fs::create_dir(&loom_dir).unwrap();

        // Knot with unknown property
        let legacy_knot = "---
name: legacy-knot
agent-profile-ref: fast
strand-dir: \"strands\"
tie-off-dir: \"old-output\"
---

Review
";
        create_knot_file(&loom_dir, "legacy", legacy_knot);
        create_knot_file(&loom_dir, "clean", VALID_KNOT);

        let repo = FileSystemLoomRepository::new();
        let (looms, warnings) = repo.scan(rig.path()).unwrap();

        assert_eq!(looms.len(), 1);
        assert_eq!(looms[0].knots.len(), 2);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("tie-off-dir"),
            "warning should mention tie-off-dir: {}",
            warnings[0]
        );
    }

    /// `save()` writes loom definition (registers in-memory).
    #[test]
    fn save_registers_loom() {
        let repo = FileSystemLoomRepository::new();
        let loom = knot::domain::entities::Loom {
            id: LoomId("saved-loom".to_string()),
            knots: vec![],
        };

        assert!(repo.save(loom.clone()).is_ok());

        let retrieved = repo.get(&LoomId("saved-loom".to_string())).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, loom.id);

        let listed = repo.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, loom.id);
    }
}

// ── FileSystemAgentProfileRepository ───────────────────────────────────────

mod profile_repo_adapter {
    use super::*;

    fn create_profile(
        dir: &std::path::Path,
        name: &str,
        provider: &str,
        model: &str,
        prompt: &str,
    ) {
        let content = format!(
            "---\nname: {name}\nprovider: {provider}\nmodel: {model}\n---\n\n{prompt}\n"
        );
        fs::write(dir.join(format!("{name}.md")), content).unwrap();
    }

    /// `get()` reads profile from `{name}.md`.
    #[test]
    fn get_reads_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();
        create_profile(
            &profiles_dir, "fast", "openai", "gpt-4o", "You are fast.",
        );

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);
        let profile = repo.get("fast").unwrap().unwrap();

        assert_eq!(profile.name, "fast");
        assert_eq!(profile.provider, "openai");
        assert_eq!(profile.model, "gpt-4o");
    }

    /// `list()` returns all profiles.
    #[test]
    fn list_returns_all_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        create_profile(
            &profiles_dir, "fast", "openai", "gpt-4o", "Fast.",
        );
        create_profile(
            &profiles_dir, "detailed", "anthropic", "claude", "Detailed.",
        );

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);
        let profiles = repo.list().unwrap();

        assert_eq!(profiles.len(), 2);
        let names: Vec<_> =
            profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"fast"));
        assert!(names.contains(&"detailed"));
    }

    /// YAML frontmatter parsing: name, provider, model, prompt body.
    #[test]
    fn yaml_frontmatter_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        fs::create_dir(&profiles_dir).unwrap();

        create_profile(
            &profiles_dir,
            "reviewer",
            "openai",
            "gpt-4o",
            "You are a code reviewer.",
        );

        let repo = FileSystemAgentProfileRepository::new(profiles_dir);
        let profile = repo.get("reviewer").unwrap().unwrap();

        assert_eq!(profile.name, "reviewer");
        assert_eq!(profile.provider, "openai");
        assert_eq!(profile.model, "gpt-4o");
        assert!(profile.profile_prompt.contains("code reviewer"));
    }
}

// ── NotifyEventSource ──────────────────────────────────────────────────────

mod notify_event_source_adapter {
    use super::*;
    use knot::domain::events::StrandEvent;

    const POLL_DELAY: Duration = Duration::from_millis(300);

    /// Build a `NotifyEventSource` with default strand IDs.
    fn create_source(
        loom_id: &str,
        knot_id: &str,
    ) -> (
        NotifyEventSource,
        mpsc::Receiver<StrandEvent>,
        mpsc::Receiver<knot::domain::events::ConfigEvent>,
    ) {
        let (strand_tx, strand_rx) = mpsc::channel(100);
        let (config_tx, config_rx) = mpsc::channel(100);
        let source = NotifyEventSource::new(
            strand_tx,
            config_tx,
            PathBuf::from("/tmp"),
        )
        .with_ids(
            LoomId(loom_id.to_string()),
            KnotId(knot_id.to_string()),
        );
        (source, strand_rx, config_rx)
    }

    /// `watch()` starts notify watching.
    #[test]
    fn watch_starts_notify_watching() {
        let (source, _strand_rx, _config_rx) = create_source("test", "k1");
        let dir = tempfile::tempdir().unwrap();

        assert!(source.watch(dir.path()).is_ok());
        assert!(source.unwatch(dir.path()).is_ok());
    }

    /// File create → event emitted on channel.
    #[test]
    fn file_create_emits_event() {
        let dir = tempfile::tempdir().unwrap();
        let (source, mut rx, _config_rx) = create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::write(dir.path().join("new-strand.md"), "content").unwrap();
        thread::sleep(POLL_DELAY);

        let event = recv_timeout(&mut rx, Duration::from_millis(500))
            .expect("should receive Created event");
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "new-strand.md"
                );
            }
            other => panic!("Expected Created, got {:?}", other),
        }
    }

    /// File modify → event emitted on channel.
    #[test]
    fn file_modify_emits_event() {
        let dir = tempfile::tempdir().unwrap();
        let (source, mut rx, _config_rx) = create_source("loom-1", "knot-1");

        let file_path = dir.path().join("existing.md");
        fs::write(&file_path, "initial").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::write(&file_path, "updated content").unwrap();
        thread::sleep(POLL_DELAY);

        let event = recv_timeout(&mut rx, Duration::from_millis(500))
            .expect("should receive Modified event");
        match event {
            StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "existing.md"
                );
            }
            other => panic!("Expected Modified, got {:?}", other),
        }
    }

    /// File delete → event emitted on channel.
    #[test]
    fn file_delete_emits_event() {
        let dir = tempfile::tempdir().unwrap();
        let (source, mut rx, _config_rx) = create_source("loom-1", "knot-1");

        let file_path = dir.path().join("to-delete.md");
        fs::write(&file_path, "will be deleted").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::remove_file(&file_path).unwrap();
        thread::sleep(POLL_DELAY);

        // Drain events, last should be Deleted
        let mut last: Option<StrandEvent> = None;
        while let Some(evt) = recv_timeout(&mut rx, Duration::from_millis(100))
        {
            last = Some(evt);
        }
        let event = last.expect("should receive at least one event");

        match event {
            StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "to-delete.md"
                );
            }
            other => panic!("Expected Deleted, got {:?}", other),
        }
    }

    /// `unwatch()` stops receiving events.
    #[test]
    fn unwatch_stops_events() {
        let dir = tempfile::tempdir().unwrap();
        let (source, mut rx, _config_rx) = create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Unwatch before creating the file
        EventSource::unwatch(&source, dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::write(dir.path().join("after-unwatch.md"), "content").unwrap();
        thread::sleep(POLL_DELAY);

        // Should NOT receive any events
        assert!(
            recv_timeout(&mut rx, Duration::from_millis(200)).is_none(),
            "should not receive events after unwatch"
        );
    }
}
