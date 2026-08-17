# Phase 1: Rig Git Initialisation

## Status

Completed

## Summary

Added `GitVersioningPort::ensure_rig_repo(rig_dir)` — the idempotent,
non-fatal rig git invariant — and enforced it at startup. On every
`run_startup()` (after rig-dir creation, before watcher registration):

- the rig directory becomes its own git repository (`git init` when
  `rig/.git` is absent; **no `.gitignore` written into the rig** — the
  rig is source-only), and
- when the project root (parent of the rig dir) is inside a git repo,
  a **marked** `<rig-basename>/` entry is appended to the parent
  `.gitignore` (idempotent) — unless the rig is already tracked by the
  parent, in which case Knot logs the exact
  `git rm -r --cached <basename>/` command instead of editing (untrack
  stays a user decision; the exclusion self-applies on a later startup
  once untracked).

Knot never commits the rig git — the user does, manually. All failure
modes (no git binary, git init failure, no parent repo, rig at the
filesystem root) are non-fatal: warning logged, startup continues.

## Changes

### Port (`src/application/ports.rs`)

- `GitVersioningPort` gains `ensure_rig_repo(rig_dir: &Path)
  -> Result<(), PortError>` with the full contract in the doc comment
  (idempotent, non-fatal, no `.gitignore` in the rig, tracked rig →
  warn without editing).
- The contract-test `MockGitVersioningPort` gains a no-op impl.

### Test fixtures (`src/application/usecases/test_fixtures.rs`)

- The public `MockGitVersioningPort` gains a no-op `ensure_rig_repo`
  (all `ProcessStrand` tests and integration helpers keep compiling
  unchanged).

### Adapter (`src/adapters/outbound/git_versioner.rs`)

- `FileSystemGitVersioner` gains a `git_binary` field (default `"git"`)
  plus a `with_git_binary()` constructor — used by tests to simulate a
  git-absent environment.
- New `run_git()` helper (returns `Option<Output>`; `None` when the
  binary cannot be spawned); `is_git_repo()` refactored onto it.
- `GITIGNORE_MARKER` constant — the marked comment written above the
  entry in the parent `.gitignore`.
- `ensure_rig_repo` implementation:
  1. `git init` in the rig dir when `rig/.git` is absent (idempotent;
     existing repos are never clobbered).
  2. When the project root is inside a git repo:
     - no-op when the `.gitignore` already contains the entry or the
       marker (idempotent),
     - tracked rig (index entries, including gitlinks — detected via
       `git ls-files -- <basename>`) → log the manual untrack command,
       **no edit**,
     - otherwise append the marked `<basename>/` entry (creates
       `.gitignore` if missing, preserves existing content).
  3. All failure paths log a warning and return `Ok(())`.
- 8 new unit tests: fresh init (no `.gitignore` in the rig), idempotent
  re-run (entry + marker exactly once), existing `.gitignore` content
  preserved, named-rig basename (`dev-rig/`), no-op when parent is not
  a repo, tracked rig → warning without edit, git binary absent
  (graceful), existing rig git not clobbered (sentinel file).

### Composition (`src/server.rs`)

- `AppContext.git_versioning: Arc<dyn GitVersioningPort>` — wired in
  `build_app_context` (project root = parent of the rig dir, same
  resolution as before).
- `run_startup()` calls `ensure_rig_repo(rig_dir)` after rig-dir +
  config-file creation and **before** DiscoverLooms/watcher
  registration; a failure logs `WARNING: rig git init: …` and
  continues.
- `spawn_process_strand_loop` no longer constructs its own versioner —
  it reuses the composition-root instance (single versioner per
  process).
- New test: `test_startup_ensures_rig_git_repository` — `run_startup`
  initialises `rig/.git`, writes no `.gitignore` into the rig, and a
  second startup is idempotent.

## Verified git behaviour (2026-08-17, sandbox)

- `git ls-files -- <basename>` detects both regular tracked files under
  the dir and gitlink entries (mode 160000) — the tracked check covers
  the "existing" scenario and staged-but-uncommitted gitlinks.
- `git rev-parse --git-dir` from the project root (parent of the rig
  dir) never finds the nested rig repo — the exclusion check is not
  confused by the rig's own git.
- `git add -A` in a parent fails fatally (`'rig/' does not have a
  commit checked out`) only when the **entire** `rig/` is untracked,
  un-ignored, and contains a `.git` with no commits. Fresh projects
  never hit this: Knot writes the `.gitignore` entry in the same
  startup, and git skips ignored dirs entirely.
- Transitional state (rig tracked by parent, nested `.git` present,
  not yet untracked): new files under `rig/` stage as regular files —
  `git add -A` succeeds; the parent keeps committing rig changes until
  the user runs the one-time untrack (Knot warns on every startup).

## Verification

- End-to-end (manual, `target/debug/knot` in temp projects):
  - **Fresh** git project: `rig/.git` created; no `.gitignore` in the
    rig; parent `.gitignore` contains the marked `rig/` entry; log
    order shows rig-init + exclusion before watcher registration.
  - **Pre-tracked** rig (this repo's case): `rig/.git` created; parent
    `.gitignore` untouched; log carries the exact
    `git rm -r --cached rig/` command.
  - **Self-healing**: after manual `git rm -r --cached rig/`, the next
    startup appends the marked entry (verified file content).

## Test Results

```
cargo test --no-fail-fast
TOTAL passed: 989 failed: 1
```

The single failure —
`domain::events::tests::build_listener_context_prompt_includes_do_not_edit_guidance`
— is **pre-existing on clean HEAD** (verified in phase 0 via stash) and
unrelated to this phase (asserts prompt template text in
`src/domain/events.rs`, no git involvement).

## Notes / Deferred

- Git versioner rig-awareness (`git reset -q -- <rig-dir>` after
  `git add -A` in `commit()`, gitlink/stale-file integration tests) is
  Phase 2.
- Legacy-layout auto-migration is Phase 3.
- Skills, glossary, docs, and the `knot-update` 0.31.0 changelog (incl.
  the documented one-time manual untrack step for existing projects)
  are Phase 4.
- The rig git starts empty — its first commit is the user's. `knot
  share` zip output is unchanged (zips looms + profiles from the rig).
