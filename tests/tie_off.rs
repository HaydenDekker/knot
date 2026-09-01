//! Application-level integration tests for tie-off output.
//!
//! Verifies tie-off path structure, append-mode history, and
//! markdown section formatting by constructing `ProcessStrand` with
//! `TrackingTieOffSink` and `MockTieOffSink`.
//!
//! No `start_knot()` calls, no `TEST_MUTEX`, no PATH manipulation —
//! all ports are mocked, tests run fully parallel, and complete in
//! sub-millisecond time.

mod helpers;

use std::path::PathBuf;
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::application::ports::AgentOutput;
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandPath, TieOffStatus,
};

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

/// Build a `StrandEvent::Created` for the given loom/knot/strand.
fn created_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> knot::domain::events::StrandEvent {
    knot::domain::events::StrandEvent::Created {
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

/// Create a real strand file on disk (needed for Created/Modified events
/// which check file existence via `StrandPath::should_process`).
fn create_strand_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Tie-off path structure ───────────────────────────────────────────────

/// Tie-off is written to the correct path under the runtime root.
///
/// Path structure: `tie-offs/<rig-basename>/{loom-id}/tie-off-{knot-id}.md`
#[test]
fn tie_off_written_to_correct_path() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_appends,
        tie_off_content,
        agent_runner: _captured,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Verify tie-off path matches expected pattern
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    let tie_off = &appends[0];

    // Path should be: /tie-offs/rig/review-loom/tie-off-review.md
    // (builder rig dir is /rig → runtime root /tie-offs/rig)
    let path_str = tie_off.path.0.display().to_string();
    assert!(
        path_str.contains("tie-offs/rig/review-loom/tie-off-review.md"),
        "tie-off path should contain loom and knot ID: {}",
        path_str
    );

    // Content map should have the same path
    let content = tie_off_content.lock().unwrap();
    assert!(
        content.contains_key(&path_str),
        "tie-off content map should have entry at path: {}",
        path_str
    );
}

/// Tie-off path includes the knot ID (not loom ID) in the filename.
///
/// With multiple knots in one loom, each gets its own tie-off file.
#[test]
fn tie_off_path_includes_knot_id() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![
        build_knot("knot-a"),
        build_knot("knot-b"),
    ]);
    let runner = success_runner("output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_appends,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "knot-a", strand_path))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    let path_str = appends[0].path.0.display().to_string();
    assert!(
        path_str.contains("tie-off-knot-a.md"),
        "tie-off filename should include knot-a ID: {}",
        path_str
    );
}

// ── Tie-off append mode ──────────────────────────────────────────────────

/// Multiple runs append to the same tie-off sink, producing a history
/// of agent outputs.
#[test]
fn tie_off_append_mode_history() {
    let dir = tempfile::tempdir().unwrap();
    let strand1 = create_strand_file(&dir, "feature1.md", "feature 1");
    let strand2 = create_strand_file(&dir, "feature2.md", "feature 2");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review v1");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_appends,
        tie_off_content,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    // First strand
    use_case.execute(created_event("review-loom", "review", strand1))
        .unwrap();

    // Second strand — same knot, same tie-off path
    use_case.execute(created_event("review-loom", "review", strand2))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 2, "should have 2 tie-off appends");
    assert!(
        appends[0].content.contains("review v1"),
        "first append should contain agent output"
    );
    assert!(
        appends[1].content.contains("review v1"),
        "second append should contain agent output"
    );

    // Content map tracks the latest write (same path, second overwrites)
    let content = tie_off_content.lock().unwrap();
    let latest = content
        .get("/tie-offs/rig/review-loom/tie-off-review.md")
        .expect("tie-off path should be in content map");
    assert!(
        latest.contains("review v1"),
        "tie-off content should contain agent output"
    );
}

/// Tie-off append tracks different content from different strands.
#[test]
fn tie_off_append_different_strands() {
    let dir = tempfile::tempdir().unwrap();
    let strand1 = create_strand_file(&dir, "feature1.md", "feature 1");
    let strand2 = create_strand_file(&dir, "feature2.md", "feature 2");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_appends,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand1))
        .unwrap();
    use_case.execute(created_event("review-loom", "review", strand2))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 2, "should have 2 appends");

    // Each append has different strand_path in its metadata
    assert!(
        appends[0].strand_path.as_deref() == Some("feature1.md")
            || appends[0].strand_path.as_ref().map(|s| s.contains("feature1")) == Some(true),
        "first append should reference feature1.md"
    );
    assert!(
        appends[1].strand_path.as_deref() == Some("feature2.md")
            || appends[1].strand_path.as_ref().map(|s| s.contains("feature2")) == Some(true),
        "second append should reference feature2.md"
    );
}

// ── Context extraction (tie-off read_content) ────────────────────────────

/// For Deleted events, the tie-off sink's `read_content` is called to
/// extract previous processing history.
#[test]
fn delete_event_context_extraction_reads_tieoff() {
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("review output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_content,
        agent_runner: captured,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    // Pre-populate tie-off with previous processing history
    {
        let mut content = tie_off_content.lock().unwrap();
        content.insert(
            "/tie-offs/rig/review-loom/tie-off-review.md".to_string(),
            concat!(
                "## review triggered by Created strands/feature.md\n",
                "Timestamp: 2026-06-05T10:00:00Z\n",
                "---\n",
                "Initial review content",
            )
            .to_string(),
        );
    }

    // Process delete event
    let event = knot::domain::events::StrandEvent::Deleted {
        loom_id: LoomId("review-loom".to_string()),
        knot_id: KnotId("review".to_string()),
        strand_path: StrandPath(PathBuf::from("strands/feature.md")),
    };
    use_case.execute(event).unwrap();

    // Verify the captured execution context has history from tie-off
    let ctx = captured.get_captured_ctx()
        .expect("ctx should be captured");

    assert!(
        ctx.prompt.contains("Previous processing history"),
        "prompt should contain history extracted from tie-off"
    );
    assert!(
        ctx.prompt.contains("Initial review content"),
        "prompt should contain content from tie-off"
    );
    assert!(
        ctx.prompt.contains("This file was deleted"),
        "prompt should contain deletion notice"
    );
}

/// Tie-off status is `Produced` on successful agent execution.
#[test]
fn tie_off_status_produced_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("ok output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_appends,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends[0].status, TieOffStatus::Produced);
}

/// Tie-off contains metadata: knot_name, event_type, strand_path.
#[test]
fn tie_off_contains_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "my-feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        tie_off_appends,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    let tie_off = &appends[0];

    assert_eq!(
        tie_off.knot_name.as_deref(),
        Some("review"),
        "tie-off should have knot_name"
    );
    assert!(
        tie_off.event_type.as_deref() == Some("Created"),
        "tie-off should have event_type = Created"
    );
    assert!(
        tie_off.strand_path.as_ref().map(|s| s.contains("my-feature.md")) == Some(true),
        "tie-off should reference the strand path"
    );
}
// ── Acceptance: session line on disk with the real tie-off sink (plan 071) ──

/// Success runner whose output metadata carries a pi session ID.
fn success_runner_with_session(output: &str, sid: &str) -> Arc<MockAgentRunner> {
    use knot::application::ports::AgentInvocationMetadata;
    Arc::new(MockAgentRunner::new(Ok(AgentOutput {
        stdout: output.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: Some(AgentInvocationMetadata {
            session_id: Some(sid.to_string()),
            token_usage: None,
            compactions: vec![],
        }),
    })))
}

/// Build a `StrandEvent::Modified` for the given loom/knot/strand.
fn modified_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> knot::domain::events::StrandEvent {
    knot::domain::events::StrandEvent::Modified {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Acceptance: with the real `FileSystemTieOffSink`, a run whose
/// metadata carries a session ID produces an on-disk tie-off containing
/// `session: <id>` between `Timestamp:` and `---`.
#[test]
fn acceptance_real_sink_writes_session_line_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let rig_dir = dir.path().join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner_with_session("output", "accept-sess");

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, runner)
            .with_real_tie_off_sink(rig_dir)
            .build();

    use_case
        .execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Runtime root: <tmp>/tie-offs/rig/review-loom/tie-off-review.md
    let file_path = dir
        .path()
        .join("tie-offs/rig/review-loom/tie-off-review.md");
    let content = std::fs::read_to_string(&file_path)
        .unwrap_or_else(|e| panic!("tie-off file should exist at {file_path:?}: {e}"));

    let lines: Vec<&str> = content.lines().collect();
    let ts = lines
        .iter()
        .position(|l| l.starts_with("Timestamp: "))
        .expect("Timestamp line");
    let sep = lines
        .iter()
        .position(|l| *l == "---")
        .expect("--- separator");
    assert_eq!(
        lines[ts + 1],
        "session: accept-sess",
        "session line must follow Timestamp on disk: {content}"
    );
    assert!(ts + 1 < sep, "session line must precede ---: {content}");
}

/// Acceptance: stdio-style run (no metadata) → on-disk file contains no
/// `session:` line and its header block matches today's exact shape:
/// header / Timestamp / --- with nothing in between.
#[test]
fn acceptance_real_sink_no_session_line_without_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let rig_dir = dir.path().join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("output");

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, runner)
            .with_real_tie_off_sink(rig_dir)
            .build();

    use_case
        .execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let file_path = dir
        .path()
        .join("tie-offs/rig/review-loom/tie-off-review.md");
    let content = std::fs::read_to_string(&file_path).unwrap();

    assert!(
        !content.contains("session:"),
        "no session line without metadata: {content}"
    );
    // Header block keeps today's exact shape.
    let lines: Vec<&str> = content.lines().collect();
    assert!(
        lines[0].starts_with("## review triggered by Created "),
        "header line unchanged: {content}"
    );
    assert!(
        lines[1].starts_with("Timestamp: "),
        "timestamp directly after header: {content}"
    );
    assert_eq!(lines[2], "---", "separator directly after timestamp: {content}");
    assert!(content.contains("output"), "body present: {content}");
}

/// Acceptance: two appended sections (each with a session ID) both
/// parse via `tieoff_parser::parse_sections` — the session line lands in
/// the section body, header and timestamp stay structured. This pins
/// the "parser unchanged" decision.
#[test]
fn acceptance_real_sink_append_sections_still_parse() {
    let dir = tempfile::tempdir().unwrap();
    let rig_dir = dir.path().join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner_with_session("first output", "accept-sess");

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, runner)
            .with_real_tie_off_sink(rig_dir)
            .build();

    use_case
        .execute(created_event("review-loom", "review", strand_path.clone()))
        .unwrap();
    use_case
        .execute(modified_event("review-loom", "review", strand_path))
        .unwrap();

    let file_path = dir
        .path()
        .join("tie-offs/rig/review-loom/tie-off-review.md");
    let content = std::fs::read_to_string(&file_path).unwrap();

    let sections =
        knot::domain::tieoff_parser::parse_sections(&content);
    assert_eq!(
        sections.len(),
        2,
        "both sections should parse: {content}"
    );
    for section in &sections {
        assert_eq!(section.knot_name, "review");
        assert!(
            section.timestamp.starts_with("20"),
            "timestamp still structured: {:?}",
            section.timestamp
        );
    }
    assert_eq!(sections[0].event_type, "Created");
    assert_eq!(sections[1].event_type, "Modified");
    // The session line is carried in the body of each section — the
    // parser is deliberately unchanged (plan 071, Target 4).
    assert!(
        sections[0].body.contains("session: accept-sess"),
        "session line belongs to the section body: {:?}",
        sections[0].body
    );
    assert!(
        sections[1].body.contains("session: accept-sess"),
        "second section body carries it too: {:?}",
        sections[1].body
    );
}
