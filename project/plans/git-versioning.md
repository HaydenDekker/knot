# Plan: Git Versioning — Automatic Commit History for Agent Work

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md).

It implements Story 9 (Git Versioning), providing a permanent, auditable record of agent work through automatic git commits. This complements the file-based rollback feature by giving users standard git tools (`git log`, `git revert`, `git diff`) for long-term history and recovery.

## Problem

When knots process strands, the agent may modify project files (via tools) and write tie-off output. Currently, these changes accumulate in the working tree with no structured history — there's no way to see *what* a knot did on each run, *when* it did it, or *revert* a bad run using standard tools. The loom-log tracks events, but doesn't version the actual file changes.

## Target

Each knot run produces a static git commit in the project root. The commit message identifies the loom, knot, strand, and event type. The commit body contains the tie-off output (current response). Opt-out per-knot via `git-versioned: false` in frontmatter. Gracefully skips if not a git repo.

## Implementation Status: ⬜ Draft | 🔄 Active | ✅ Complete

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

- [ ] Add `git_versioned: bool` field to `Knot` entity (default `true`)
- [ ] Add `git_versioned: Option<bool>` to `KnotFile` struct (parsed from frontmatter)
- [ ] Add `#[serde(rename = "git-versioned")]` field to `RawFrontmatter`
- [ ] Update `parse()` to extract field, defaulting to `true` when absent
- [ ] Update `KnotFile` → `Knot` conversion (wherever it happens) to pass the field
- [ ] Tests:
  - `knot_file_with_git_versioned_true` — parses `git-versioned: true`
  - `knot_file_with_git_versioned_false` — parses `git-versioned: false`
  - `knot_file_without_git_versioned_defaults_true` — absent field → `true`
  - `knot_serialization_roundtrip_with_git_versioned` — JSON round-trip preserves field
  - `knot_file_roundtrip_with_git_versioned` — generate → parse round-trip

### Phase 1: Application — GitVersioningPort and ProcessStrand Integration

**Hex Layer:** Application (Port + Use Case)

- [ ] Add `GitVersioningPort` trait to `application/ports.rs`:
  ```rust
  pub trait GitVersioningPort: Send + Sync {
      fn commit(
          &self,
          loom_id: &LoomId,
          knot_id: &KnotId,
          strand_path: &StrandPath,
          event_type: &str,
          tie_off_content: &str,
      ) -> Result<(), PortError>;
  }
  ```
- [ ] Add `PortError::GitCommitFailed(String)` variant with `Display` impl
- [ ] Add mock `GitVersioningPort` to port tests module
- [ ] Add `git_versioning_port: Arc<dyn GitVersioningPort>` to `ProcessStrand`
- [ ] In `ProcessStrand::execute()`, after tie-off write (before log completion):
  - Check `knot.git_versioned` — if `false`, skip
  - Call `git_versioning_port.commit()` with tie-off content (the current response, not full file)
  - On error: log warning, do NOT fail processing (strand still marked completed)
- [ ] Tests:
  - `git_versioning_port_contract` — trait is object-safe, methods callable
  - `process_strand_calls_git_port_on_success` — mock port receives commit call
  - `process_strand_skips_git_when_disabled` — `git_versioned: false` → no commit call
  - `process_strand_continues_on_git_error` — mock port returns error → processing succeeds
  - `port_error_git_commit_display` — error Display impl
  - Update existing ProcessStrand tests to include mock git port

### Phase 2: Outbound Adapter — FileSystemGitVersioner

**Hex Layer:** Outbound Adapter

- [ ] Create `src/adapters/outbound/git_versioner.rs`
- [ ] Implement `FileSystemGitVersioner` with subprocess approach:
  - Uses `std::process::Command` to run `git` (avoids `git2` C dependency)
  - `git add -A` (stages all changes)
  - `git commit -m "<message>"` (with body from tie-off)
  - Commit message format: `knot: <knot-id> — processed <strand-name> (<event-type>)`
  - Commit body: tie-off content (truncated to ~1000 lines if excessive)
- [ ] Handle edge cases:
  - Not a git repo: check `git rev-parse --git-dir` first, skip if fails
  - Git unavailable (binary not found): skip, log warning
  - Git commit fails (e.g. no config): skip, log warning
  - Uncommitted changes from other sources: `git add -A` includes everything — that's the desired behaviour (all agent work captured together)
- [ ] Tests:
  - `git_versioner_creates_commit_in_git_repo` — temp dir with init'd git, verify commit
  - `git_versioner_skips_when_not_git_repo` — temp dir without git init, no error
  - `git_versioner_commit_message_format` — message includes loom, knot, strand, event
  - `git_versioner_commit_body_contains_tieoff` — body has tie-off content
  - `git_versioner_trait_object_safe` — trait is object-safe
  - `git_versioner_multiple_commits_in_sequence` — simulates series of knot runs

### Phase 3: Composition Root — Wire the Adapter

**Hex Layer:** Composition Root

- [ ] In `src/server.rs` (or wherever ProcessStrand is constructed):
  - Create `FileSystemGitVersioner` with project root (parent of `base_dir`)
  - Pass as `Arc<dyn GitVersioningPort>` to `ProcessStrand::new()`
- [ ] Update `AppConfig` if needed (likely not — project root derives from `base_dir`)
- [ ] Update `tests/composition.rs` to verify new wiring compiles

### Phase 4: Integration Tests — Full Pipeline with Git

**Hex Layer:** Integration

- [ ] In `tests/pipeline.rs` (or new `tests/git_versioning.rs`):
  - `event_pipeline_creates_git_commit` — full pipeline with git repo, verify commit exists and has correct message/body
  - `event_pipeline_skips_git_when_disabled` — knot with `git-versioned: false`, no commit created
  - `event_pipeline_continues_without_git` — no git repo, processing succeeds normally
- [ ] Update `tests/helpers.rs`:
  - `init_git_repo(dir)` — `git init` + `git config` in temp dir
  - Helper to read latest commit message/body

## Notes

- **Subprocess vs git2:** Using `std::process::Command` to call `git` avoids the `git2` crate's libgit2 C dependency. Knot is a local tool — users will have git installed. The subprocess approach also handles git config, hooks, and credentials transparently.
- **Commit scope:** `git add -A` stages all changes. This captures tie-off + any working tree modifications the agent made via tools in a single atomic commit — the desired behaviour for "everything the agent did in this run."
- **Opt-out semantics:** Default is `true` (opt-out). Some review-based knots may be bundled with other changes and don't need their own commit. The `git-versioned: false` frontmatter field handles this.
- **Graceful degradation:** Git versioning must never fail the processing pipeline. If git is unavailable, not configured, or the commit fails, a warning is logged but the strand still processes and completes normally.
