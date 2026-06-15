# Plan: Git Versioning — Automatic Commit History for Agent Work

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md).

It implements Story 10 (Git Versioning), providing a permanent, auditable record of agent work through automatic git commits. This complements the file-based rollback feature by giving users standard git tools (`git log`, `git revert`, `git diff`) for long-term history and recovery.

## Problem

When knots process strands, the agent may modify project files (via tools) and write tie-off output. Currently, these changes accumulate in the working tree with no structured history — there's no way to see *what* a knot did on each run, *when* it did it, or *revert* a bad run using standard tools. The loom-log tracks events, but doesn't version the actual file changes.

## Target

Each knot run produces a static git commit in the project root. The commit message identifies the loom, knot, strand, and event type. The commit body contains the tie-off output (current response). Opt-out per-knot via `git-versioned: false` in frontmatter. Gracefully skips if not a git repo.

## Implementation Status: ✅ Complete (2026-06-14)

## Completion Notes
- All 5 phases (0–4) complete, 17 new unit tests + 3 integration tests
- `FileSystemGitVersioner` uses `std::process::Command` (no C dependency)
- Graceful degradation: skips if not a git repo, git unavailable, or commit fails
- Wired in composition root via `start_event_pipeline` in `src/server.rs`
- Version bumped to 0.5.0 (MINOR — new backwards-compatible feature)

### Bugfix: Commit ordering (2026-06-15)

`KnotCompleted` and `StrandProcessed` loom-log entries were written *after* the `git add -A` commit, so they were left uncommitted and picked up by the *next* agent invoke instead. Fixed by reordering `ProcessStrand::execute()` so the commit runs after both loom-log appends (the log adapter already flushes on each write).

## Existing Tests
| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tests/pipeline.rs` | Full event pipeline (source → debounce → ProcessStrand → tie-off) | ✅ Green — 3 tests |
| `tests/agent_integration.rs` | Agent CLI execution, error handling | ✅ Green — 4 tests |
| `tests/composition.rs` | Composition root wiring | ✅ Green — 3 tests |
| `tests/helpers.rs` | Shared test infrastructure (mock agents, HTTP helpers) | ✅ Green — reused |
| `src/domain/knot_file.rs::tests` | KnotFile parsing, frontmatter validation | ✅ Green — 24 tests |
| `src/domain/entities.rs::tests` | Entity construction, serialization | ✅ Green — 14 tests |
| `src/application/ports.rs::tests` | Port contracts, supporting types | ✅ Green — 14 tests |
| `src/application/usecases.rs::tests` | Use case unit tests with mock ports | ✅ Green — 20+ tests |
| `src/adapters/outbound/tieoff_sink.rs::tests` | Tie-off write, append, parent dirs | ✅ Green — 6 tests |

## Test Gaps
- No tests for git versioning at any layer
- No tests for `git_versioned` field on `Knot`/`KnotFile`
- No tests for graceful skip when not a git repo
- No integration test covering the full pipeline with git versioning enabled

## Phases

### Phase 0: Domain — Add `git_versioned` to Knot and KnotFile

**Hex Layer:** Domain

- [x] Add `git_versioned: bool` field to `Knot` entity (default `true`)
- [x] Add `git_versioned: Option<bool>` to `KnotFile` struct (parsed from frontmatter)
- [x] Add `#[serde(rename = "git-versioned")]` field to `RawFrontmatter`
- [x] Update `parse()` to extract field, defaulting to `true` when absent
- [x] Update `KnotFile` → `Knot` conversion (wherever it happens) to pass the field
- [x] Tests:
  - [x] `knot_file_with_git_versioned_true` — parses `git-versioned: true`
  - [x] `knot_file_with_git_versioned_false` — parses `git-versioned: false`
  - [x] `knot_file_without_git_versioned_defaults_true` — absent field → `true`
  - [x] `knot_serialization_roundtrip_with_git_versioned` — JSON round-trip preserves field
  - [x] `knot_file_roundtrip_with_git_versioned` — generate → parse round-trip

### Phase 1: Application — GitVersioningPort and ProcessStrand Integration

**Hex Layer:** Application (Port + Use Case)

- [x] Add `GitVersioningPort` trait to `application/ports.rs`
- [x] Add `PortError::GitCommitFailed(String)` variant with `Display` impl
- [x] Add mock `GitVersioningPort` to port tests module
- [x] Add `git_versioning_port: Arc<dyn GitVersioningPort>` to `ProcessStrand`
- [x] In `ProcessStrand::execute()`, after tie-off write (before log completion):
  - Check `knot.git_versioned` — if `false`, skip
  - Call `git_versioning_port.commit()` with tie-off content (the current response, not full file)
  - On error: log warning, do NOT fail processing (strand still marked completed)
- [x] Tests:
  - [x] `git_versioning_port_contract` — trait is object-safe, methods callable
  - [x] `process_strand_calls_git_port_on_success` — mock port receives commit call
  - [x] `process_strand_skips_git_when_disabled` — `git_versioned: false` → no commit call
  - [x] `process_strand_continues_on_git_error` — mock port returns error → processing succeeds
  - [x] `port_error_git_commit_display` — error Display impl
  - [x] Update existing ProcessStrand tests to include mock git port

### Phase 2: Outbound Adapter — FileSystemGitVersioner

**Hex Layer:** Outbound Adapter

- [x] Create `src/adapters/outbound/git_versioner.rs`
- [x] Implement `FileSystemGitVersioner` with subprocess approach
- [x] Handle edge cases (not a git repo, git unavailable, commit failures — all non-fatal)
- [x] Tests:
  - [x] `git_versioner_creates_commit_in_git_repo`
  - [x] `git_versioner_skips_when_not_git_repo`
  - [x] `git_versioner_commit_message_format`
  - [x] `git_versioner_commit_body_contains_tieoff`
  - [x] `git_versioner_trait_object_safe`
  - [x] `git_versioner_multiple_commits_in_sequence`

### Phase 3: Composition Root — Wire the Adapter

**Hex Layer:** Composition Root

- [x] In `src/server.rs` (or wherever ProcessStrand is constructed):
  - [x] Create `FileSystemGitVersioner` with project root (parent of `base_dir`)
  - [x] Pass as `Arc<dyn GitVersioningPort>` to `ProcessStrand::new()`
- [x] Update `AppConfig` if needed (likely not — project root derives from `base_dir`)
- [x] Update `tests/composition.rs` to verify new wiring compiles

### Phase 4: Integration Tests — Full Pipeline with Git

**Hex Layer:** Integration

- [x] In `tests/pipeline.rs` (or new `tests/git_versioning.rs`):
  - [x] `event_pipeline_creates_git_commit` — full pipeline with git repo, verify commit exists and has correct message/body
  - [x] `event_pipeline_skips_git_when_disabled` — knot with `git-versioned: false`, no commit created
  - [x] `event_pipeline_continues_without_git` — no git repo, processing succeeds normally
- [x] Update `tests/helpers.rs`:
  - [x] `init_git_repo(dir)` — `git init` + `git config` in temp dir
  - [x] `get_latest_commit(dir)` — read latest commit subject/body
  - [x] `count_commits(dir)` — count total commits in repo

## Notes

- **Subprocess vs git2:** Using `std::process::Command` to call `git` avoids the `git2` crate's libgit2 C dependency. Knot is a local tool — users will have git installed. The subprocess approach also handles git config, hooks, and credentials transparently.
- **Commit scope:** `git add -A` stages all changes. This captures tie-off + any working tree modifications the agent made via tools in a single atomic commit — the desired behaviour for "everything the agent did in this run."
- **Opt-out semantics:** Default is `true` (opt-out). Some review-based knots may be bundled with other changes and don't need their own commit. The `git-versioned: false` frontmatter field handles this.
- **Graceful degradation:** Git versioning must never fail the processing pipeline. If git is unavailable, not configured, or the commit fails, a warning is logged but the strand still processes and completes normally.
