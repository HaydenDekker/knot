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

/// Filesystem-backed git versioning adapter.
///
/// Uses `std::process::Command` to run `git` directly — avoids the
/// `git2` C dependency. All failures are non-fatal: if git is
/// unavailable, the directory is not a repo, or the commit fails for
/// any other reason, the method returns `Ok(())` and logs a warning.
pub struct FileSystemGitVersioner {
    /// Project root where git commands should run.
    repo_root: std::path::PathBuf,
}

impl FileSystemGitVersioner {
    /// Create a new versioner targeting `repo_root`.
    pub fn new(repo_root: std::path::PathBuf) -> Self {
        Self { repo_root }
    }

    /// Check if `repo_root` is inside a git repository.
    ///
    /// Returns `Ok(())` if `git rev-parse --git-dir` succeeds, or
    /// `Err(PortError::GitCommitFailed)` if it fails.
    fn is_git_repo(&self) -> Result<(), PortError> {
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| {
                crate::adapters::logging::log_config_event(
                    "git_versioner",
                    &format!("git binary not found: {e}"),
                );
                PortError::GitCommitFailed(format!(
                    "git binary not found: {e}"
                ))
            })?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            crate::adapters::logging::log_config_event(
                "git_versioner",
                &format!("not a git repo: {stderr}"),
            );
            Err(PortError::GitCommitFailed(format!(
                "not a git repo: {stderr}"
            )))
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
        let versioner =
            FileSystemGitVersioner::new(dir.path().to_path_buf());

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
        let versioner =
            FileSystemGitVersioner::new(dir.path().to_path_buf());

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
        let versioner =
            FileSystemGitVersioner::new(dir.path().to_path_buf());

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
        let versioner =
            FileSystemGitVersioner::new(dir.path().to_path_buf());

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
        let versioner =
            FileSystemGitVersioner::new(std::path::PathBuf::from("/tmp"));
        let _obj: &dyn GitVersioningPort = &versioner;
    }

    #[test]
    fn git_versioner_multiple_commits_in_sequence() {
        let dir = setup_git_repo();
        let versioner =
            FileSystemGitVersioner::new(dir.path().to_path_buf());

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
}
