//! Application-level integration tests for the event processing pipeline.
//!
//! Verifies the full strand-processing pipeline (events → ProcessStrand → tie-off)
//! by constructing `ProcessStrand` with mocked ports. No `start_knot()` calls,
//! no `TEST_MUTEX`, no PATH manipulation — all ports are mocked, tests run
//! fully parallel, and complete in sub-millisecond time.
//!
//! Debounce timing and notify event coalescing are verified by the
//! `NotifyEventSource` adapter test in `tests/adapters.rs`.
//! The multi-knot shared-directory unwatch fix is verified by a composition
//! smoke test in `tests/smoke.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use knot::application::ports::{
    AgentOutput, AgentRunner, PortError,
};
use knot::application::store::LoomStore;
use knot::application::usecases::ProcessStrand;
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffStatus,
};
use knot::domain::events::{LoomEvent, StrandEvent};
use knot::RigAgentConfig;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a knot with the given ID and "fast" profile ref.
fn build_knot(id: &str) -> Knot {
    build_knot_with_profile(id, "fast")
}

/// Build a loom with the given ID and knots.
fn build_loom(id: &str, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

/// Build the ProcessStrand use case with all mocks wired up.
///
/// Returns a tuple of (use_case, log_events, tie_off_appends, rig_events,
/// tie_off_content, agent_runner, git_port, file_checker).
#[allow(clippy::type_complexity)]
fn build_process_strand(
    loom: Loom,
    agent_runner: Arc<MockAgentRunner>,
) -> (
    ProcessStrand,
    Arc<Mutex<Vec<LoomEvent>>>,
    Arc<Mutex<Vec<TieOff>>>,
    Arc<Mutex<Vec<knot::domain::events::RigLogEvent>>>,
    Arc<Mutex<HashMap<String, String>>>,
    Arc<MockAgentRunner>,
    Arc<MockGitVersioningPort>,
    Arc<MockStrandFileChecker>,
) {
    let store = LoomStore::new();
    store.register(loom);

    let (log_port, log_events) = MockLoomLogPort::new();
    let (tie_off_sink, tie_off_appends, tie_off_content) =
        TrackingTieOffSink::new();
    let (rig_log, rig_events) = MockRigLogPort::new();
    let (git_port, _git_commits) = MockGitVersioningPort::new();
    let git_port = Arc::new(git_port);
    let file_checker = Arc::new(MockStrandFileChecker::new());
    let event_dispatcher = Arc::new(MockEventDispatcher::default());

    let profile_repo = Arc::new(MockProfileRepository {
        profiles: Arc::new(Mutex::new(HashMap::from_iter([
            ("fast".to_string(), default_profile()),
        ]))),
    });

    let use_case = ProcessStrand::new(
        store.clone(),
        Arc::new(log_port),
        agent_runner.clone() as Arc<dyn AgentRunner>,
        Arc::new(tie_off_sink),
        RigAgentConfig::default_config(),
        PathBuf::from("/rig"),
        profile_repo,
        Arc::new(rig_log),
        git_port.clone(),
        file_checker.clone(),
        event_dispatcher,
    );

    (
        use_case,
        log_events,
        tie_off_appends,
        rig_events,
        tie_off_content,
        agent_runner,
        git_port,
        file_checker,
    )
}

/// Build a `StrandEvent::Created` for the given loom/knot/strand.
fn created_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> StrandEvent {
    StrandEvent::Created {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Build a `StrandEvent::Modified` for the given loom/knot/strand.
fn modified_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> StrandEvent {
    StrandEvent::Modified {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Build a `StrandEvent::Deleted` for the given loom/knot/strand.
fn deleted_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> StrandEvent {
    StrandEvent::Deleted {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Build a successful agent output mock runner.
fn success_runner(output: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Ok(AgentOutput {
        stdout: output.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    })))
}

/// Build a failing agent execution mock runner.
fn failure_runner(message: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Err(
        PortError::AgentExecutionFailed {
            message: message.to_string(),
            session_id: None,
        },
    )))
}

/// Create a real strand file on disk (needed for Created/Modified events
/// which check file existence via `StrandPath::should_process`).
fn create_strand_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Pipeline: process flow tests ─────────────────────────────────────────

/// Full pipeline flow: Created event → KnotProcessing → agent run → tie-off
/// → KnotCompleted → StrandProcessed.
#[test]
fn pipeline_processes_strand_create() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "new feature request");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    let event = created_event("review-loom", "review", strand_path);

    let result = use_case.execute(event);
    assert!(result.is_ok());

    // Verify loom-log has KnotCompleted event
    let events = log_events.lock().unwrap();
    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed, "should have KnotCompleted event");

    // Verify tie-off was written
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    assert!(
        appends[0].content.contains("review output"),
        "tie-off should contain agent output"
    );
    assert_eq!(appends[0].status, TieOffStatus::Produced);
}

/// Modified event triggers reprocessing: same pipeline flow as Created.
#[test]
fn pipeline_reprocesses_on_strand_modify() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "v1");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    // First: Created event
    use_case.execute(created_event("review-loom", "review", strand_path.clone()))
        .unwrap();

    // Second: Modified event (reprocessing)
    use_case.execute(modified_event("review-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();
    let completed_count = events.iter()
        .filter(|e| matches!(e, LoomEvent::KnotCompleted { .. }))
        .count();
    assert!(
        completed_count >= 2,
        "should have at least 2 KnotCompleted events (create + modify), got {}",
        completed_count
    );

    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 2, "should have 2 tie-off appends");
}

/// Deleted event triggers the pipeline: KnotCompleted and StrandProcessed
/// are logged, tie-off is appended.
#[test]
fn pipeline_handles_strand_delete() {
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    let event = deleted_event(
        "review-loom",
        "review",
        PathBuf::from("strands/feature.md"),
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();
    let has_processed = events.iter()
        .any(|e| matches!(e, LoomEvent::StrandProcessed { .. }));
    assert!(has_processed, "should have StrandProcessed event after delete");

    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed, "should have KnotCompleted event after delete");
}

/// Pipeline processes non-`.md` text files (`.rs`, `.json`, etc.) normally.
///
/// Verifies that arbitrary text extensions trigger full pipeline processing.
#[test]
fn pipeline_processes_non_md_text_files() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "lib.rs", "fn main() { println!(\"hello\"); }");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("rust review output");

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        _captured, _git, _file_checker) = build_process_strand(loom, runner);
    let event = created_event("review-loom", "review", strand_path);
    let result = use_case.execute(event);
    assert!(result.is_ok());

    // Verify KnotCompleted (not StrandIgnored)
    let events = log_events.lock().unwrap();
    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed, "should have KnotCompleted for .rs file");

    let has_ignored = events.iter()
        .any(|e| matches!(e, LoomEvent::StrandIgnored { .. }));
    assert!(
        !has_ignored,
        "should NOT have StrandIgnored for .rs file"
    );

    // Verify tie-off was written
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have tie-off for .rs file");
    assert!(
        appends[0].content.contains("rust review output"),
        "tie-off should contain agent output for .rs file"
    );
}

/// Binary files produce StrandIgnored in loom-log, no agent invocation,
/// no tie-off. Text files in the same loom process normally.
#[test]
fn pipeline_ignores_binary_files_and_processes_text_files() {
    let dir = tempfile::tempdir().unwrap();
    let text_path = create_strand_file(&dir, "notes.txt", "some plain text notes");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        _captured, _git, file_checker) = build_process_strand(loom, runner);

    // --- Binary file: should be ignored ---
    let binary_path = dir.path().join("data.bin");
    std::fs::write(&binary_path, vec![0x00u8, 0x01, 0x02, 0xFF]).unwrap();
    file_checker.mark_binary(&binary_path);

    let binary_event = created_event("review-loom", "review", binary_path.clone());
    use_case.execute(binary_event).unwrap();

    {
        let events = log_events.lock().unwrap();
        let has_ignored = events.iter()
            .any(|e| matches!(e, LoomEvent::StrandIgnored { .. }));
        assert!(has_ignored, "should have StrandIgnored for binary file");

        // Verify StrandIgnored has correct reason
        let ignored = events.iter().find(|e| {
            matches!(e, LoomEvent::StrandIgnored { .. })
        });
        if let Some(LoomEvent::StrandIgnored { reason, .. }) = ignored {
            assert!(
                reason.contains("binary"),
                "reason should mention binary, got: {}",
                reason
            );
        }

        // Verify no KnotProcessing for binary file
        let processing_events: Vec<_> = events.iter()
            .filter(|e| matches!(e, LoomEvent::KnotProcessing { .. }))
            .collect();
        assert!(
            processing_events.is_empty(),
            "should have no KnotProcessing events for binary file"
        );
    }

    // --- Text file: should process normally ---
    let text_event = created_event("review-loom", "review", text_path);
    use_case.execute(text_event).unwrap();

    {
        let events = log_events.lock().unwrap();
        let has_completed = events.iter()
            .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
        assert!(has_completed, "should have KnotCompleted for .txt file");
    }

    let appends = tie_off_appends.lock().unwrap();
    // 1 append for text file (binary produces 0)
    assert_eq!(appends.len(), 1, "should have 1 tie-off for text file");
}

/// The pipeline handles agent execution errors gracefully: KnotFailed +
/// StrandProcessed with error, no panic.
#[test]
fn pipeline_handles_agent_failure() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = failure_runner("agent crash");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    let event = created_event("review-loom", "review", strand_path);
    let result = use_case.execute(event);
    assert!(result.is_ok(), "execute should return Ok even on agent failure");

    let events = log_events.lock().unwrap();
    let has_failed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotFailed { .. }));
    assert!(has_failed, "should have KnotFailed event");

    // KnotFailed contains error detail
    for event in events.iter() {
        if let LoomEvent::KnotFailed { error, .. } = event {
            assert!(
                error.contains("crash"),
                "error should contain crash detail"
            );
        }
    }
}

// ── Loom-log event sequence ────────────────────────────────────────────

/// Loom-log contains the full event sequence for strand processing:
/// KnotProcessing → KnotCompleted → StrandProcessed.
#[test]
fn loom_log_contains_full_event_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Should have: KnotProcessing, KnotCompleted, StrandProcessed
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotProcessing { .. })),
        "should have KnotProcessing"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::StrandProcessed { .. })),
        "should have StrandProcessed"
    );

    // Verify order: KnotProcessing → KnotCompleted → StrandProcessed
    let processing_idx = events.iter().position(|e| {
        matches!(e, LoomEvent::KnotProcessing { .. })
    }).unwrap();
    let completed_idx = events.iter().position(|e| {
        matches!(e, LoomEvent::KnotCompleted { .. })
    }).unwrap();
    let processed_idx = events.iter().position(|e| {
        matches!(e, LoomEvent::StrandProcessed { .. })
    }).unwrap();

    assert!(
        processing_idx < completed_idx && completed_idx < processed_idx,
        "events should be in order: KnotProcessing < KnotCompleted < StrandProcessed"
    );
}

// ── Delete Event Context Tests ─────────────────────────────────────────

/// When a strand is deleted, the agent's prompt should contain a deletion
/// notice and the previous processing history from the tie-off file.
#[test]
fn delete_event_agent_receives_context() {
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, _log_events, _tie_off_appends, _rig_events,
        tie_off_content, captured, _git, _checker) =
        build_process_strand(loom, runner);

    // Pre-populate tie-off with previous processing history
    {
        let mut content = tie_off_content.lock().unwrap();
        content.insert(
            "/rig/tie-offs/review-loom/tie-off-review.md".to_string(),
            concat!(
                "## review triggered by Created strands/feature.md\n",
                "Timestamp: 2026-06-05T10:00:00Z\n",
                "---\n",
                "Initial review content",
            )
            .to_string(),
        );
    }

    // Now process the delete event
    let event = deleted_event(
        "review-loom",
        "review",
        PathBuf::from("strands/feature.md"),
    );
    use_case.execute(event).unwrap();

    // Verify the captured execution context has the right prompt
    let ctx = captured.get_captured_ctx()
        .expect("ctx should be captured");

    // Verify deletion notice is present
    assert!(
        ctx.prompt.contains("This file was deleted"),
        "prompt should contain deletion notice:\n{}",
        ctx.prompt
    );

    // Verify previous processing history is included
    assert!(
        ctx.prompt.contains("Previous processing history"),
        "prompt should contain previous processing history:\n{}",
        ctx.prompt
    );

    // Verify the strand name appears in the history
    assert!(
        ctx.prompt.contains("feature.md"),
        "prompt should reference the strand path:\n{}",
        ctx.prompt
    );

    // Verify a trigger line from previous processing appears
    assert!(
        ctx.prompt.contains("triggered by Created")
            || ctx.prompt.contains("triggered by Modified"),
        "prompt should contain a trigger entry from previous processing:\n{}",
        ctx.prompt
    );
}

/// When a strand is deleted, the agent should execute successfully without
/// errors about missing files (no `@file` reference for deleted events).
#[test]
fn delete_event_agent_skips_missing_file() {
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        captured, _git, _checker) = build_process_strand(loom, runner);

    // Process a delete event for a file that doesn't exist on disk
    // (Deleted events skip the file existence check)
    let event = deleted_event(
        "review-loom",
        "review",
        PathBuf::from("strands/feature.md"),
    );
    use_case.execute(event).unwrap();

    // Verify no @file reference in agent args
    let ctx = captured.get_captured_ctx()
        .expect("ctx should be captured");
    let has_at_ref = ctx.agent_config.extra_args
        .iter()
        .any(|arg| arg.starts_with('@'));
    assert!(
        !has_at_ref,
        "Deleted events must NOT contain @file reference"
    );

    // Verify KnotCompleted (not KnotFailed)
    let events = log_events.lock().unwrap();
    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed, "should have KnotCompleted for delete");

    let has_failed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotFailed { .. }));
    assert!(
        !has_failed,
        "should have no KnotFailed (agent should not error on missing file)"
    );
}

/// When a tie-off file has many entries for multiple strands, deleting one
/// strand should only inject the last 5 entries for that strand (not all).
#[test]
fn delete_event_large_tieoff_bounded_context() {
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let (use_case, _log_events, _tie_off_appends, _rig_events,
        tie_off_content, captured, _git, _checker) =
        build_process_strand(loom, runner);

    // Build a tie-off with many entries for target.md interleaved
    // with entries for other strands
    let mut tie_off_entries = Vec::new();
    for i in 1..=8 {
        tie_off_entries.push(format!(
            "## review triggered by Modified strands/target.md\n\
             Timestamp: 2026-06-05T{:02}:00:00Z\n\
             ---\n\
             Target review v{}",
            i, i
        ));
        // Interleave with other strand entries
        if i <= 3 {
            tie_off_entries.push(format!(
                "## review triggered by Created strands/other{}.md\n\
                 Timestamp: 2026-06-05T{:02}:30:00Z\n\
                 ---\n\
                 Other {} review",
                i, i, i
            ));
        }
    }

    {
        let mut content = tie_off_content.lock().unwrap();
        content.insert(
            "/rig/tie-offs/review-loom/tie-off-review.md".to_string(),
            tie_off_entries.join("\n---\n"),
        );
    }

    // Delete target.md
    let event = deleted_event(
        "review-loom",
        "review",
        PathBuf::from("strands/target.md"),
    );
    use_case.execute(event).unwrap();

    let ctx = captured.get_captured_ctx()
        .expect("ctx should be captured");

    // Count how many target.md references appear in the prompt
    let target_refs = ctx.prompt.matches("target.md").count();

    // The prompt contains:
    // - "Strand: target.md" (strand label) — 1 ref
    // - up to 5 history headers with the strand path — up to 5 refs
    // - the trigger line at the bottom with the strand path — 1 ref
    // So at most 1 + 5 + 1 = 7 references.
    assert!(
        target_refs <= 7,
        "prompt should contain at most 7 references to target.md \
         (strand label + last 5 history entries + trigger line), \
         got {}. Prompt:\n{}",
        target_refs,
        ctx.prompt
    );

    // The prompt should NOT contain other strand names
    assert!(
        !ctx.prompt.contains("other1.md"),
        "prompt should NOT contain other1.md (only target strand history):\n{}",
        ctx.prompt
    );
    assert!(
        !ctx.prompt.contains("other2.md"),
        "prompt should NOT contain other2.md:\n{}",
        ctx.prompt
    );
    assert!(
        !ctx.prompt.contains("other3.md"),
        "prompt should NOT contain other3.md:\n{}",
        ctx.prompt
    );

    // Verify the deletion notice is present
    assert!(
        ctx.prompt.contains("This file was deleted"),
        "prompt should contain deletion notice:\n{}",
        ctx.prompt
    );
}

// ── Strand Skip Tests (temp file / missing file) ────────────────────────

/// Known temp files (sedXXXXXXX) are silently skipped by the pipeline.
/// No loom-log entry, no agent invocation, no error.
#[test]
fn pipeline_silently_skips_known_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let normal_path = create_strand_file(&dir, "normal.md", "normal content");
    let temp_path = dir.path().join("sedXXXXXXX");
    // Create temp file so it exists (Created event checks existence)
    std::fs::write(&temp_path, "temp content").unwrap();

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("slow output");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    // Process the temp file event
    use_case.execute(created_event("review-loom", "review", temp_path)).unwrap();

    // Process normal file
    use_case.execute(created_event("review-loom", "review", normal_path)).unwrap();

    let events = log_events.lock().unwrap();
    let types: Vec<_> = events.iter()
        .filter_map(|e| {
            match e {
                LoomEvent::KnotCompleted { .. } => Some("KnotCompleted"),
                LoomEvent::KnotFailed { .. } => Some("KnotFailed"),
                LoomEvent::StrandSkipped { .. } => Some("StrandSkipped"),
                LoomEvent::StrandIgnored { .. } => Some("StrandIgnored"),
                LoomEvent::KnotProcessing { .. } => Some("KnotProcessing"),
                LoomEvent::StrandProcessed { .. } => Some("StrandProcessed"),
                _ => None,
            }
        })
        .collect();

    // normal.md should have been processed normally
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted for normal.md. Events: {:?}",
        types
    );

    // Key verifications for the temp file scenario:
    // - No KnotFailed (temp file handling should not produce errors)
    // - No StrandSkipped (known temp files are silently skipped)
    // - No StrandIgnored (temp files are not binary files)
    assert!(
        !types.contains(&"KnotFailed"),
        "should NOT have KnotFailed (temp file should not error). Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"StrandSkipped"),
        "should NOT have StrandSkipped for known temp file. Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"StrandIgnored"),
        "should NOT have StrandIgnored for known temp file. Events: {:?}",
        types
    );
}

/// Unknown missing files produce a StrandSkipped loom-log entry.
#[test]
fn pipeline_logs_strand_skipped_for_unknown_missing_file() {
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("output");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git, _checker) = build_process_strand(loom, runner);

    // Create event for a file that does NOT exist on disk
    let missing_path = PathBuf::from("strands/some_missing_file.md");
    use_case.execute(created_event("review-loom", "review", missing_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Verify StrandSkipped is present
    let skipped_events: Vec<_> = events.iter()
        .filter(|e| matches!(e, LoomEvent::StrandSkipped { .. }))
        .collect();
    assert!(
        !skipped_events.is_empty(),
        "should have StrandSkipped event for missing file. Events: {:?}",
        events
    );

    // Verify the StrandSkipped event has the correct reason
    if let LoomEvent::StrandSkipped { reason, .. } = skipped_events[0] {
        assert!(
            reason.contains("missing"),
            "reason should mention missing file, got: {}",
            reason
        );
    }

    // No KnotFailed should appear
    let has_failed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotFailed { .. }));
    assert!(
        !has_failed,
        "should NOT have KnotFailed for missing file"
    );
}
