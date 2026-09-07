//! Filesystem-backed implementation of `GitVersioningPort`.
//!
//! Uses `git` subprocess commands to create commits after successful
//! knot runs. Gracefully handles non-git directories, missing git
//! binary, and unconfigured repos.

use std::process::Command;

use crate::application::ports::{GitVersioningPort, PortError};
use crate::domain::entities::{KnotId, LoomId, StrandPath};

/// Maximum number of lines in the commit body (tie-off content).
/// Prevents excessively large commit messages.
const MAX_BODY_LINES: usize = 1000;

/// Marker comment appended above the rig entry in the parent repo's
/// `.gitignore` — makes the exclusion idempotent (Knot never appends
/// it twice) and visible to humans.
const GITIGNORE_MARKER: &str =
    "# knot: rig has its own git repository (excluded from project repo)";

/// Filesystem-backed git versioning adapter.
///
/// Uses `std::process::Command` to run `git` directly — avoids the
/// `git2` C dependency. All failures are non-fatal: if git is
/// unavailable, the directory is not a repo, or the commit fails for
/// any other reason, the method returns `Ok(())` and logs a warning.
///
/// The versioner is rig-aware: after staging everything with
/// `git add -A`, it unstages the rig directory (`git reset -q --
/// <rig-dir>`) so the nested rig repository (gitlink) or any tracked
/// rig leftovers can never enter a project commit.
pub struct FileSystemGitVersioner {
    /// Project root where git commands should run.
    repo_root: std::path::PathBuf,
    /// Rig directory to keep out of project commits (typically
    /// `<repo_root>/<rig-basename>`).
    rig_dir: std::path::PathBuf,
    /// Git binary name (overridable in tests to simulate git absence).
    git_binary: String,
}

impl FileSystemGitVersioner {
    /// Create a new versioner targeting `repo_root`.
    ///
    /// `rig_dir` is the rig directory to exclude from project commits
    /// — it must resolve to a path at or below `repo_root` for the
    /// exclusion to apply.
    pub fn new(repo_root: std::path::PathBuf, rig_dir: std::path::PathBuf) -> Self {
        Self {
            repo_root,
            rig_dir,
            git_binary: "git".to_string(),
        }
    }

    /// Create a versioner with an explicit git binary name.
    ///
    /// Used in tests to simulate a git-absent environment.
    pub fn with_git_binary(
        repo_root: std::path::PathBuf,
        rig_dir: std::path::PathBuf,
        git_binary: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            rig_dir,
            git_binary: git_binary.into(),
        }
    }

    /// Run a git command in `dir`. Returns `None` if the git binary
    /// cannot be spawned.
    fn run_git(
        &self,
        dir: &std::path::Path,
        args: &[&str],
    ) -> Option<std::process::Output> {
        Command::new(&self.git_binary)
            .args(args)
            .current_dir(dir)
            .output()
            .ok()
    }

    /// Unstage the rig directory after `git add -A` so it can never
    /// enter a project commit.
    ///
    /// Runs `git reset -q -- <rig-dir-relative>` in the repo root. The
    /// reset covers both rig-leakage scenarios: a staged gitlink (the
    /// rig's nested repo, mode 160000) and tracked leftover files from
    /// before the rig got its own repository. Returns `Ok(())` when
    /// the reset succeeds or is skipped (rig not under the repo root,
    /// or degenerate rig-is-repo-root case where an empty pathspec
    /// would unstage everything); returns
    /// `Err(PortError::GitCommitFailed)` when the reset fails, so the
    /// caller aborts the commit rather than committing a staged rig.
    fn unstage_rig(&self) -> Result<(), PortError> {
        let relative = match self.rig_dir.strip_prefix(&self.repo_root) {
            Ok(relative) => relative,
            Err(_) => {
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    &format!(
                        "rig dir {} is not under repo root {} — rig exclusion skipped",
                        self.rig_dir.display(),
                        self.repo_root.display(),
                    ),
                );
                return Ok(());
            }
        };
        if relative.as_os_str().is_empty() {
            // Degenerate case: rig dir IS the repo root. Resetting an
            // empty pathspec would unstage everything — skip.
            crate::adapters::logging::log_config_event(
                "git_versioner",
                "rig dir is the repo root — rig exclusion skipped",
            );
            return Ok(());
        }
        let rel = relative.to_string_lossy();
        match self.run_git(&self.repo_root, &["reset", "-q", "--", &rel]) {
            Some(output) if output.status.success() => Ok(()),
            Some(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    &format!("git reset -- {rel} failed: {stderr}"),
                );
                Err(PortError::GitCommitFailed(format!(
                    "git reset -- {rel} failed: {stderr}"
                )))
            }
            None => {
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    "git binary not available — cannot verify rig exclusion",
                );
                Err(PortError::GitCommitFailed(
                    "git binary not available — cannot verify rig exclusion".to_string(),
                ))
            }
        }
    }

    /// Check if `repo_root` is inside a git repository.
    ///
    /// Returns `Ok(())` if `git rev-parse --git-dir` succeeds, or
    /// `Err(PortError::GitCommitFailed)` if it fails.
    fn is_git_repo(&self) -> Result<(), PortError> {
        match self.run_git(&self.repo_root, &["rev-parse", "--git-dir"]) {
            Some(output) if output.status.success() => Ok(()),
            Some(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    &format!("not a git repo: {stderr}"),
                );
                Err(PortError::GitCommitFailed(format!(
                    "not a git repo: {stderr}"
                )))
            }
            None => {
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    "git binary not found",
                );
                Err(PortError::GitCommitFailed(
                    "git binary not found".to_string(),
                ))
            }
        }
    }

    /// Truncate content to at most `MAX_BODY_LINES` lines.
    fn truncate_body(content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() <= MAX_BODY_LINES {
            content.to_string()
        } else {
            let truncated: String =
                lines[..MAX_BODY_LINES].iter().copied().collect::<Vec<&str>>()
                    .join("\n");
            format!(
                "{}\n\n... (truncated, {} more lines omitted)",
                truncated,
                lines.len() - MAX_BODY_LINES
            )
        }
    }

    /// Build the commit message subject line.
    ///
    /// Format: `knot: <knot-id> — processed <strand-name> (<event-type>)`
    fn build_subject(
        knot_id: &KnotId,
        strand_path: &StrandPath,
        event_type: &str,
    ) -> String {
        let strand_name = strand_path
            .0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| strand_path.0.display().to_string());
        format!(
            "knot: {} — processed {} ({})",
            knot_id.0, strand_name, event_type
        )
    }
}

impl GitVersioningPort for FileSystemGitVersioner {
    fn commit(
        &self,
        _loom_id: &LoomId,
        knot_id: &KnotId,
        strand_path: &StrandPath,
        event_type: &str,
        tie_off_content: &str,
    ) -> Result<(), PortError> {
        // 1. Check if this is a git repo
        if self.is_git_repo().is_err() {
            // Not a git repo — skip gracefully
            return Ok(());
        }

        // 2. Stage all changes
        let add_result = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| {
                PortError::GitCommitFailed(format!("git add failed: {e}"))
            })?;

        if !add_result.status.success() {
            let stderr = String::from_utf8_lossy(&add_result.stderr);
            crate::adapters::logging::log_config_event(
                "git_versioner",
                &format!("git add failed: {stderr}"),
            );
            return Err(PortError::GitCommitFailed(format!(
                "git add failed: {stderr}"
            )));
        }

        // 2b. Unstage the rig — the rig has its own git repository and
        //     must never enter a project commit (gitlink or tracked
        //     leftovers). A reset failure aborts the commit: committing
        //     with a staged rig is exactly what this guard prevents.
        self.unstage_rig()?;

        // 3. Build commit message
        let subject = Self::build_subject(knot_id, strand_path, event_type);
        let body = Self::truncate_body(tie_off_content);
        let full_message = format!("{subject}\n\n{body}");

        // 4. Commit
        let commit_result = Command::new("git")
            .args(["commit", "-m"])
            .arg(&full_message)
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| {
                PortError::GitCommitFailed(format!("git commit failed: {e}"))
            })?;

        if !commit_result.status.success() {
            // Check if nothing to commit (no changes)
            let stderr = String::from_utf8_lossy(&commit_result.stderr);
            if stderr.contains("nothing added to commit")
                || stderr.contains("no changes added")
                || stderr.contains("nothing to commit")
            {
                // No changes — not an error, skip silently
                return Ok(());
            }

            // Other failure (e.g. no git user configured)
            crate::adapters::logging::log_config_event(
                "git_versioner",
                &format!("git commit failed: {stderr}"),
            );
            Err(PortError::GitCommitFailed(format!(
                "git commit failed: {stderr}"
            )))
        } else {
            let stdout = String::from_utf8_lossy(&commit_result.stdout);
            crate::adapters::logging::log_config_event(
                "git_versioner",
                &format!(
                    "committed: {}",
                    knot_id.0,
                ),
            );
            let _ = stdout; // captured for future use (e.g. short hash)
            Ok(())
        }
    }

    fn ensure_rig_repo(&self, rig_dir: &std::path::Path) -> Result<(), PortError> {
        // 1. Rig's own git repository (idempotent). No `.gitignore` is
        //    written into the rig — the rig directory is source-only.
        if !rig_dir.join(".git").exists() {
            match self.run_git(rig_dir, &["init"]) {
                Some(output) if output.status.success() => {
                    crate::adapters::logging::log_config_event(
                        "git_versioner",
                        "initialised rig git repository",
                    );
                }
                Some(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    crate::adapters::logging::log_config_event(
                        "git_versioner",
                        &format!("git init in rig dir failed: {stderr}"),
                    );
                }
                None => {
                    crate::adapters::logging::log_config_event(
                        "git_versioner",
                        "git binary not available — rig git repository not initialised",
                    );
                }
            }
        }

        // 2. Parent exclusion — only meaningful when the project root
        //    (parent of the rig dir) is inside a git repository.
        let Some(project_root) = rig_dir.parent() else {
            return Ok(()); // degenerate: rig dir has no parent
        };
        match self.run_git(project_root, &["rev-parse", "--git-dir"]) {
            Some(output) if output.status.success() => {}
            _ => return Ok(()), // no git binary or parent not a git repo
        }

        let basename = rig_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "rig".to_string());
        let bare_entry = format!("{basename}/");
        let entry = format!("/{basename}/");
        let gitignore = project_root.join(".gitignore");

        // Classify the existing content. An anchored entry means the file
        // is already in the correct form — idempotent no-op. A bare entry
        // is the pre-0.41.1 form (unanchored patterns match at any depth,
        // so `rig/` also ignored `tie-offs/rig/`) and gets force-migrated
        // in place below.
        let mut content = std::fs::read_to_string(&gitignore).unwrap_or_default();
        let lines: Vec<&str> = content.lines().collect();
        let has_anchored = lines.iter().any(|l| l.trim() == entry);
        let has_bare = lines.iter().any(|l| l.trim() == bare_entry);
        if has_anchored {
            return Ok(());
        }

        // Tracked by the parent? Do not edit — log the manual untrack
        // command. Untracking rewrites the project's index, so it stays
        // a user decision; the exclusion (or migration) is applied on a
        // later startup once the rig is untracked.
        if let Some(ls) =
            self.run_git(project_root, &["ls-files", "--", basename.as_str()])
        {
            if ls.status.success()
                && !String::from_utf8_lossy(&ls.stdout).trim().is_empty()
            {
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    &format!(
                        "rig dir '{basename}' is tracked by the parent repo — run `git rm -r --cached {entry}` (from {}) to untrack it; the .gitignore exclusion is applied once untracked",
                        project_root.display(),
                    ),
                );
                return Ok(());
            }
        }

        // Force-migrate a pre-existing bare entry in place: rewrite only
        // that line to the anchored form — the marker and every other
        // line are preserved verbatim. Convergence is one-time: the
        // anchored line short-circuits on every later startup.
        if has_bare {
            let migrated = lines
                .iter()
                .map(|l| {
                    if l.trim() == bare_entry {
                        entry.as_str()
                    } else {
                        l
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Err(e) = std::fs::write(&gitignore, format!("{migrated}\n")) {
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    &format!("failed to write {}: {e}", gitignore.display()),
                );
                return Ok(());
            }
            crate::adapters::logging::log_config_event(
                "git_versioner",
                &format!(
                    "migrated unanchored rig exclusion to '{entry}' in {}",
                    gitignore.display()
                ),
            );
            return Ok(());
        }

        // A marker without an entry line means the user removed the
        // entry — leave the file alone.
        if lines.iter().any(|l| l.trim() == GITIGNORE_MARKER) {
            return Ok(());
        }

        // Append the marked entry (create the file if missing).
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("{GITIGNORE_MARKER}\n{entry}\n"));
        if let Err(e) = std::fs::write(&gitignore, content) {
            crate::adapters::logging::log_config_event(
                "git_versioner",
                &format!("failed to write {}: {e}", gitignore.display()),
            );
            return Ok(());
        }
        crate::adapters::logging::log_config_event(
            "git_versioner",
            &format!(
                "excluded '{entry}' from parent repo via {}",
                gitignore.display()
            ),
        );
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    /// Helper: create a temp directory with a git repo initialized and
    /// configured (user.name/email set so commits work).
    fn setup_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-b", "main"]);
        run_git(dir.path(), &["config", "user.email", "test@test.com"]);
        run_git(dir.path(), &["config", "user.name", "Test User"]);
        dir
    }

    /// Helper: create a temp directory without git init.
    fn setup_plain_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Helper: run a git command in the given directory.
    fn run_git(dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args.iter().map(|s| s.to_string()))
            .current_dir(dir)
            .output()
            .expect("git should be available on test system")
    }

    /// Helper: create a file in the repo.
    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    /// Helper: get the latest commit message from a repo.
    fn get_last_commit(dir: &Path) -> (String, String) {
        let output = run_git(dir, &["log", "-1", "--format=%B"]);
        let msg = String::from_utf8_lossy(&output.stdout).to_string();
        // Split into subject (first line) and body (rest after blank line)
        let lines: Vec<&str> = msg.lines().collect();
        let subject = lines.first().map(|s| s.to_string()).unwrap_or_default();
        let body_start = msg.find("\n\n");
        let body = if let Some(pos) = body_start {
            msg[pos + 2..].trim().to_string()
        } else {
            String::new()
        };
        (subject, body)
    }

    #[test]
    fn git_versioner_creates_commit_in_git_repo() {
        let dir = setup_git_repo();
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            dir.path().join("rig"),
        );

        // Create an initial commit so the repo is in a clean state
        write_file(dir.path(), "initial.txt", "start");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "initial"]);

        // Modify a file (simulating agent work)
        write_file(dir.path(), "output.txt", "agent result");

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("k1".to_string());
        let strand =
            StrandPath(std::path::PathBuf::from("input/strand.md"));

        let result =
            versioner.commit(&loom_id, &knot_id, &strand, "Created", "body");
        assert!(result.is_ok(), "commit should succeed in git repo");

        // Verify a new commit was created
        let (subject, _body) = get_last_commit(dir.path());
        assert!(
            subject.contains("k1"),
            "subject should contain knot id: {subject}"
        );
    }

    #[test]
    fn git_versioner_skips_when_not_git_repo() {
        let dir = setup_plain_dir();
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            dir.path().join("rig"),
        );

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("k1".to_string());
        let strand =
            StrandPath(std::path::PathBuf::from("input/strand.md"));

        let result = versioner.commit(
            &loom_id,
            &knot_id,
            &strand,
            "Created",
            "body",
        );
        // Should return Ok(()) — graceful skip, not an error
        assert!(
            result.is_ok(),
            "should skip gracefully when not a git repo"
        );
    }

    #[test]
    fn git_versioner_commit_message_format() {
        let dir = setup_git_repo();
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            dir.path().join("rig"),
        );

        // Initial commit
        write_file(dir.path(), "initial.txt", "start");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "initial"]);

        // Modify a file
        write_file(dir.path(), "output.txt", "result");

        let loom_id = LoomId("review-loom".to_string());
        let knot_id = KnotId("goals-review".to_string());
        let strand =
            StrandPath(std::path::PathBuf::from("docs/goals.md"));

        let result = versioner.commit(
            &loom_id,
            &knot_id,
            &strand,
            "Modified",
            "tie-off content",
        );
        assert!(result.is_ok());

        let (subject, body) = get_last_commit(dir.path());

        // Subject format: knot: <knot-id> — processed <strand-name> (<event-type>)
        assert!(
            subject.starts_with("knot: "),
            "subject should start with 'knot: '"
        );
        assert!(
            subject.contains("goals-review"),
            "subject should contain knot id"
        );
        assert!(
            subject.contains("goals.md"),
            "subject should contain strand file name"
        );
        assert!(
            subject.contains("Modified"),
            "subject should contain event type"
        );

        // Body should contain tie-off content
        assert_eq!(
            body, "tie-off content",
            "body should contain tie-off content"
        );
    }

    #[test]
    fn git_versioner_commit_body_contains_tieoff() {
        let dir = setup_git_repo();
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            dir.path().join("rig"),
        );

        // Initial commit
        write_file(dir.path(), "initial.txt", "start");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "initial"]);

        // Modify a file
        write_file(dir.path(), "output.txt", "result");

        let loom_id = LoomId("test".to_string());
        let knot_id = KnotId("k1".to_string());
        let strand =
            StrandPath(std::path::PathBuf::from("input.md"));
        let tie_off = "Line 1 of tie-off\nLine 2 of tie-off\nLine 3";

        let result = versioner.commit(
            &loom_id,
            &knot_id,
            &strand,
            "Created",
            tie_off,
        );
        assert!(result.is_ok());

        let (_subject, body) = get_last_commit(dir.path());
        assert!(
            body.contains("Line 1 of tie-off"),
            "body should contain tie-off line 1"
        );
        assert!(
            body.contains("Line 2 of tie-off"),
            "body should contain tie-off line 2"
        );
        assert!(
            body.contains("Line 3"),
            "body should contain tie-off line 3"
        );
    }

    #[test]
    fn git_versioner_trait_object_safe() {
        let versioner = FileSystemGitVersioner::new(
            std::path::PathBuf::from("/tmp"),
            std::path::PathBuf::from("/tmp/rig"),
        );
        let _obj: &dyn GitVersioningPort = &versioner;
    }

    #[test]
    fn git_versioner_multiple_commits_in_sequence() {
        let dir = setup_git_repo();
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            dir.path().join("rig"),
        );

        // Initial commit
        write_file(dir.path(), "initial.txt", "start");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "initial"]);

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("k1".to_string());

        // First knot run
        write_file(dir.path(), "file1.md", "content 1");
        let strand1 =
            StrandPath(std::path::PathBuf::from("input/strand1.md"));
        let result1 = versioner.commit(
            &loom_id,
            &knot_id,
            &strand1,
            "Created",
            "tie-off 1",
        );
        assert!(result1.is_ok(), "first commit should succeed");

        // Second knot run
        write_file(dir.path(), "file2.md", "content 2");
        let strand2 =
            StrandPath(std::path::PathBuf::from("input/strand2.md"));
        let result2 = versioner.commit(
            &loom_id,
            &knot_id,
            &strand2,
            "Modified",
            "tie-off 2",
        );
        assert!(result2.is_ok(), "second commit should succeed");

        // Third knot run
        write_file(dir.path(), "file3.md", "content 3");
        let strand3 =
            StrandPath(std::path::PathBuf::from("input/strand3.md"));
        let result3 = versioner.commit(
            &loom_id,
            &knot_id,
            &strand3,
            "Deleted",
            "tie-off 3",
        );
        assert!(result3.is_ok(), "third commit should succeed");

        // Verify commit count (3 new + 1 initial = 4 total)
        let output = run_git(dir.path(), &["log", "--format=%H"]);
        let commit_count =
            String::from_utf8_lossy(&output.stdout).lines().count();
        assert!(
            commit_count >= 4,
            "should have at least 4 commits (1 initial + 3 knot runs), got {}",
            commit_count
        );

        // Verify latest commit message
        let (subject, body) = get_last_commit(dir.path());
        assert!(
            subject.contains("strand3.md"),
            "latest commit should reference strand3"
        );
        assert!(
            subject.contains("Deleted"),
            "latest commit should have Deleted event type"
        );
        assert_eq!(
            body, "tie-off 3",
            "latest commit body should match tie-off 3"
        );
    }

    // ── ensure_rig_repo tests ────────────────────────────────────────────

    /// Helper: temp project dir with a git repo at the root and a rig
    /// subdirectory (no git inside the rig yet).
    fn setup_project_with_rig(rig_name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = setup_git_repo();
        let rig_dir = dir.path().join(rig_name);
        fs::create_dir_all(&rig_dir).unwrap();
        (dir, rig_dir)
    }

    #[test]
    fn ensure_rig_repo_initialises_rig_git() {
        let (dir, rig_dir) = setup_project_with_rig("rig");
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        assert!(!rig_dir.join(".git").exists());

        let result = versioner.ensure_rig_repo(&rig_dir);
        assert!(result.is_ok(), "ensure_rig_repo should not fail");

        assert!(
            rig_dir.join(".git").exists(),
            "rig/.git should be created by git init"
        );
        assert!(
            !rig_dir.join(".gitignore").exists(),
            "no .gitignore should be written into the rig dir"
        );
    }

    #[test]
    fn ensure_rig_repo_is_idempotent() {
        let (dir, rig_dir) = setup_project_with_rig("rig");
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());
        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        let entry_count = content.lines().filter(|l| l.trim() == "/rig/").count();
        let marker_count = content
            .lines()
            .filter(|l| l.trim().starts_with("# knot:"))
            .count();
        assert_eq!(
            entry_count, 1,
            "entry should be appended exactly once: {content}"
        );
        assert_eq!(
            marker_count, 1,
            "marker should be appended exactly once: {content}"
        );
    }

    #[test]
    fn ensure_rig_repo_migrates_unanchored_entry() {
        let (dir, rig_dir) = setup_project_with_rig("rig");
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        // Pre-existing .gitignore in the pre-0.41.1 shape: the marker +
        // the unanchored entry as older binaries wrote it, plus a
        // user-authored sentinel line.
        let gitignore = dir.path().join(".gitignore");
        fs::write(
            &gitignore,
            format!("target/\n{GITIGNORE_MARKER}\nrig/\n"),
        )
        .unwrap();

        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        let content = fs::read_to_string(&gitignore).unwrap();
        assert!(
            content.lines().any(|l| l.trim() == "/rig/"),
            "bare entry migrated in place to the anchored form: {content}"
        );
        assert!(
            !content.lines().any(|l| l.trim() == "rig/"),
            "no bare entry remains: {content}"
        );
        assert!(
            content.lines().any(|l| l.trim() == "target/"),
            "user-authored lines preserved verbatim: {content}"
        );
        assert_eq!(
            content.lines().filter(|l| l.trim() == "/rig/").count(),
            1,
            "migration rewrites the line in place, no duplicate appended: {content}"
        );
        assert_eq!(
            content
                .lines()
                .filter(|l| l.trim() == GITIGNORE_MARKER)
                .count(),
            1,
            "marker preserved exactly once: {content}"
        );

        // Idempotent after migration: a second run changes nothing.
        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());
        assert_eq!(
            fs::read_to_string(&gitignore).unwrap(),
            content,
            "second run is a no-op once anchored"
        );
    }

    #[test]
    fn ensure_rig_repo_leaves_anchored_entry() {
        let (dir, rig_dir) = setup_project_with_rig("rig");
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        // Already in the current shape (marker + anchored entry).
        let gitignore = dir.path().join(".gitignore");
        let original = format!("target/\n{GITIGNORE_MARKER}\n/rig/\n");
        fs::write(&gitignore, &original).unwrap();

        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        assert_eq!(
            fs::read_to_string(&gitignore).unwrap(),
            original,
            "anchored entry left untouched (idempotent)"
        );
    }

    #[test]
    fn ensure_rig_repo_appends_marked_entry_preserving_existing() {
        let (dir, rig_dir) = setup_project_with_rig("rig");
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        // Pre-existing .gitignore content must be preserved.
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();

        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("target/"), "existing entries preserved");
        assert!(content.lines().any(|l| l.trim() == "/rig/"), "rig entry appended");
        assert!(
            content.contains(GITIGNORE_MARKER),
            "entry is marked: {content}"
        );
    }

    #[test]
    fn ensure_rig_repo_uses_rig_basename_for_named_rigs() {
        let (dir, rig_dir) = setup_project_with_rig("dev-rig");
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(
            content.lines().any(|l| l.trim() == "/dev-rig/"),
            "named rig should be excluded by anchored basename: {content}"
        );
    }

    #[test]
    fn ensure_rig_repo_noop_when_parent_not_git_repo() {
        let dir = setup_plain_dir();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(&rig_dir).unwrap();
        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );

        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        assert!(
            rig_dir.join(".git").exists(),
            "rig git still initialised even without a parent repo"
        );
        assert!(
            !dir.path().join(".gitignore").exists(),
            "no .gitignore created when the parent is not a git repo"
        );
    }

    #[test]
    fn ensure_rig_repo_warns_without_editing_when_rig_tracked() {
        let (dir, rig_dir) = setup_project_with_rig("rig");

        // Track a file under the rig in the parent repo (before the rig
        // gets its own git).
        let loom = rig_dir.join("review-loom");
        fs::create_dir_all(&loom).unwrap();
        write_file(&loom, "k.md", "knot");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "track rig"]);

        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );
        assert!(
            versioner.ensure_rig_repo(&rig_dir).is_ok(),
            "tracked rig is a non-fatal condition"
        );

        assert!(
            !dir.path().join(".gitignore").exists(),
            "no .gitignore edit when the rig is tracked by the parent"
        );
        assert!(
            rig_dir.join(".git").exists(),
            "the rig git itself is still initialised"
        );
    }

    #[test]
    fn ensure_rig_repo_graceful_without_git_binary() {
        let dir = setup_plain_dir();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(&rig_dir).unwrap();
        let versioner = FileSystemGitVersioner::with_git_binary(
            dir.path().to_path_buf(),
            rig_dir.clone(),
            "definitely-not-a-git-binary",
        );

        let result = versioner.ensure_rig_repo(&rig_dir);
        assert!(
            result.is_ok(),
            "git-absent environment degrades gracefully"
        );
        assert!(
            !rig_dir.join(".git").exists(),
            "no rig git created without the git binary"
        );
    }

    #[test]
    fn ensure_rig_repo_skips_existing_rig_git() {
        let (dir, rig_dir) = setup_project_with_rig("rig");

        // Pre-existing rig repo with a sentinel file inside .git.
        let git_dir = rig_dir.join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        write_file(&git_dir, "sentinel", "keep");

        let versioner = FileSystemGitVersioner::new(
            dir.path().to_path_buf(),
            rig_dir.clone(),
        );
        assert!(versioner.ensure_rig_repo(&rig_dir).is_ok());

        assert!(
            git_dir.join("sentinel").exists(),
            "existing rig git repo is not clobbered"
        );
    }
}
