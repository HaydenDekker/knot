# Phase 2: Git Versioner Rig-Awareness

## Status

Completed

## Summary

Made the git versioner rig-aware so the rig can never enter a project
commit. `FileSystemGitVersioner` now takes the rig directory at
construction, and `commit()` runs `git reset -q -- <rig-dir-relative>`
immediately after `git add -A` — unstaging the rig before the commit is
taken. This covers both rig-leakage scenarios verified in the plan:

- **Fresh rig** — `rig/` never tracked by the parent; `git add -A`
  stages it as a **gitlink** (mode 160000) → the reset removes the
  gitlink from the index.
- **Pre-tracked rig** — rig files already in the parent's history;
  `git add -A` stages modified tracked rig files and new rig files as
  regular files → the reset unstages them (they remain tracked — the
  reset does not untrack; that stays a manual user decision).

A reset failure **aborts the commit** (returns `GitCommitFailed`):
committing with a staged rig is exactly what the guard prevents, and the
knot run itself is unaffected — `ProcessStrand` treats commit errors as
non-fatal warnings.

## Changes

### Adapter (`src/adapters/outbound/git_versioner.rs`)

- `FileSystemGitVersioner` gains a `rig_dir: PathBuf` field;
  `new(repo_root, rig_dir)` and `with_git_binary(repo_root, rig_dir,
  git_binary)` now both take it. The struct doc comment records the
  rig-awareness contract.
- New `unstage_rig()` helper:
  - resolves the rig path relative to the repo root via
    `strip_prefix`;
  - **rig not under the repo root** → log + skip (nothing to unstage);
  - **rig dir IS the repo root** (degenerate) → log + skip: resetting
    an empty pathspec would unstage *everything*;
  - otherwise runs `git reset -q -- <relative>` in the repo root;
  - reset failure or unspawnable git binary → log +
    `Err(GitCommitFailed)` (the commit is aborted, not taken with a
    staged rig).
- `commit()` calls `unstage_rig()` between the `git add -A` step and
  message construction (step "2b"); all other behaviour unchanged.

### Composition (`src/server.rs`)

- `build_app_context` passes `config.rig_dir.clone()` as the versioner's
  rig dir (project root resolution unchanged — parent of the rig dir).

### Call sites

- `tests/composition.rs` — trait-object-safety compile check updated to
  the two-argument constructor.
- All 14 unit-test constructor calls in `git_versioner.rs` updated;
  tests without a rig pass a non-existent `<dir>/rig` (the reset is a
  no-op for an absent path).

### Integration tests (`tests/git_versioning.rs`, real nested repos)

New section with real `git` subprocess helpers (`git`, `git_stdout`,
`init_git_repo`, `tracked_at_head`):

- `git_commit_excludes_fresh_rig_gitlink` — parent repo + fresh rig with
  its own committed repo (a committed rig is what makes `git add -A`
  stage a gitlink rather than fail). After `commit()`: HEAD contains the
  project change and **no** `rig` entry (gitlinks are listed by
  `git ls-tree -r HEAD`, so a leaked gitlink would fail the assertion);
  nothing left staged; the rig's working tree is untouched (reset only
  unstages).
- `git_commit_excludes_pre_tracked_rig_files` — rig files committed to
  the parent before the rig got its own repo. After `commit()`: the
  project change is committed; the modified tracked rig file keeps its
  **old** committed content and remains tracked (unstaged, not
  untracked); the new rig file is not committed.
- `git_commit_skips_rig_exclusion_when_rig_is_repo_root` — degenerate
  rig-is-repo-root: the exclusion is skipped and the commit proceeds
  with all changes intact (guards against the empty-pathspec
  unstage-everything bug).

## Verified git behaviour (2026-08-17, sandbox)

- `git add -A` in a parent with a committed nested repo stages the rig
  as a gitlink (exit 0, embedded-repo warning on stderr).
- `git reset -q -- rig` removes the staged gitlink; `git status` then
  shows `?? rig/` (untracked), and the working tree is untouched.
- `git ls-tree -r HEAD --name-only` **lists committed gitlinks** — the
  "no `rig` entry at HEAD" assertion catches a leaked gitlink.

## Verification

- `cargo test --no-fail-fast` — 992 passed, 1 failed (the pre-existing
  `domain::events` prompt-text failure, unrelated).
- `cargo test --test git_versioning` — 17 passed (14 existing + 3 new).
- `cargo test --lib startup` — all 3 composition startup tests pass,
  including `test_startup_ensures_rig_git_repository` with the new
  two-argument constructor.
- `cargo clippy --all-targets` — 162 warnings, identical to HEAD
  (no new warnings).

## Test Results

```
cargo test --no-fail-fast
TOTAL passed: 992 failed: 1
```

The single failure —
`domain::events::tests::build_listener_context_prompt_includes_do_not_edit_guidance`
— is **pre-existing on clean HEAD** (documented in phases 0 and 1) and
unrelated to this phase.

## Notes / Deferred

- Legacy-layout auto-migration (`rig/tie-offs/` →
  `tie-offs/<rig-basename>/`, runtime files to the runtime root) is
  Phase 3.
- Skills, glossary, docs, and the `knot-update` 0.31.0 changelog are
  Phase 4.
- The guard is defence-in-depth on top of the Phase 1 `.gitignore`
  exclusion: fresh projects never stage the rig (ignored), the
  pre-tracked transitional state stages it until the one-time manual
  untrack, and this reset ensures no project commit ever contains rig
  content in the meantime.
