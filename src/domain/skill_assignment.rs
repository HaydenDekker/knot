//! Explicit per-session skill assignment (plan 105 — the skill-supply seam).
//!
//! Every knot session is spawned with pi's **full skill discovery set**
//! unless this seam says otherwise. The seam is a per-session, explicit
//! `--skill` list composed from three declaration surfaces and handed to
//! the runner as spawn args:
//!
//! 1. **Rig config** (`.workspace-agent-config.yaml`, key `skills`) —
//!    rig-global, declared on the **project side** (in a shim rig this
//!    file is local while the workflow is served from the external rig,
//!    so project-specific behaviour skills live here and the served rig
//!    stays architecture-neutral).
//! 2. **Profile frontmatter** (`skills:` in `profiles/{name}.md`) —
//!    every knot served by that profile.
//! 3. **Knot frontmatter** (`skills:` in the knot definition) — that
//!    knot only.
//!
//! Semantics:
//!
//! - **Additive (default)** — discovery set ∪ declared: the engine only
//!   ever *adds* `--skill` flags. An empty declaration at every surface
//!   passes neither `--skill` nor `--no-skills` — pi's normal discovery
//!   delivers the full set, unchanged (the zero-migration property).
//! - **Exclusive** (`skills-mode: exclusive`) — the engine adds
//!   `--no-skills` plus the declared set only: per-session narrowing.
//!   Opt-in; an exclusive rig that declares nothing narrows to *no*
//!   skills (a warning is logged — coherent, but never silent).
//! - **De-duplication** on the resolved path, first declaration wins
//!   (rig → profile → knot order is preserved).
//! - **Path rules** (the plan-102 instruction-link precedent): absolute
//!   paths as-is; `file://` URLs converted to paths; `~/...` expanded to
//!   `$HOME`; relative paths resolved against the declaration's root —
//!   rig-config entries against the **local** rig directory, knot/profile
//!   frontmatter against the **effective** rig directory (plan 100).
//!   `http(s)://` is refused (Knot never fetches over the network).
//!   Entries whose resolved path does not exist are **warned and
//!   skipped** — visible, never vacuous.
//!
//! Freshness: the whole chain is re-read at each new session — the
//! `skills:` lists (the rig-config key via
//! [`SkillAssignmentSource`], fresh per call; knot/profile frontmatter
//! via the existing rig-file hot reload) and the skill files themselves
//! (pi reads them at session start). Edits land on the next new session,
//! never mid-session (the plan-103 prompt-drift rule by analogy).
//!
//! The engine names no architecture, ever: it resolves paths and
//! composes flags; it interprets no skill content.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What the rig-level config declares for this project's sessions
/// (plan 105), as read fresh per new session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillDeclarations {
    /// Raw link values from the rig-config `skills` list (unresolved).
    pub rig_entries: Vec<String>,
    /// `true` when `skills-mode: exclusive` is set — narrowing mode:
    /// the session receives `--no-skills` plus the declared set only.
    pub exclusive: bool,
}

/// Per-session source of the rig-config skill declarations.
///
/// Implemented over `.workspace-agent-config.yaml` by
/// `FileSystemSkillAssignmentSource`; every call is a **fresh read** —
/// the lists are never cached from startup or a previous run, so a
/// project can add, re-point, or remove a declaration and the next
/// session picks it up with no service restart (the plan-102
/// [`PromptInstructionSource`](crate::domain::events::PromptInstructionSource)
/// precedent).
pub trait SkillAssignmentSource: Send + Sync + std::fmt::Debug {
    /// Fresh read of the rig-config `skills` list and `skills-mode`.
    /// Missing or unparseable config → [`SkillDeclarations::default`]
    /// (additive, empty — full discovery, unchanged behaviour).
    fn read_skill_declarations(&self) -> SkillDeclarations;
}

/// Resolve one skill link value against `root`.
///
/// Rules (the plan-102 instruction-link precedent): `http(s)://` and
/// `file://` non-file schemes are refused; `~/...` expands to `home`
/// (refused when `home` is `None`); absolute paths are used as-is;
/// relative paths resolve against `root`.
pub fn resolve_skill_value(
    value: &str,
    root: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Err(format!(
            "'{value}' uses http(s) — only local paths and file:// URLs are supported"
        ));
    }
    if let Some(rest) = value.strip_prefix("file://") {
        return Ok(PathBuf::from(rest));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return match home {
            Some(home) => Ok(home.join(rest)),
            None => Err(format!(
                "'{value}' uses ~/ but $HOME is not set"
            )),
        };
    }
    let path = PathBuf::from(value);
    if path.is_relative() {
        Ok(root.join(path))
    } else {
        Ok(path)
    }
}

/// Compose the per-session skill spawn args from the three declaration
/// surfaces (plan 105).
///
/// Returns `(args, warnings)`:
///
/// - `args` — CLI arguments in order: `--no-skills` first when
///   `declarations.exclusive`, then one `--skill <path>` pair per
///   de-duplicated, resolvable entry (rig → profile → knot order
///   preserved). Empty when nothing is declared and the mode is
///   additive — pi's normal discovery then delivers the full set,
///   unchanged.
/// - `warnings` — one line per refused/unresolvable/missing entry, plus
///   a note when exclusive mode narrows to nothing. The caller logs
///   these as `[KNOT][EVENT] type=CONFIG source=skill-assignment`.
///
/// De-duplication is on the **resolved path** (first declaration wins).
/// Existence is checked here: an entry whose path does not exist (file
/// or skill directory) is warned and skipped — a declaration never
/// silently evaluates to nothing.
pub fn compose_skill_cli_args(
    declarations: &SkillDeclarations,
    profile_skills: &[String],
    knot_skills: &[String],
    config_root: &Path,
    frontmatter_root: &Path,
    home: Option<&Path>,
) -> (Vec<String>, Vec<String>) {
    let mut args: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    if declarations.exclusive {
        args.push("--no-skills".to_string());
    }

    let surfaces: [(&str, &[String], &Path); 3] = [
        ("rig-config", &declarations.rig_entries, config_root),
        ("profile", profile_skills, frontmatter_root),
        ("knot", knot_skills, frontmatter_root),
    ];

    for (surface, entries, root) in surfaces {
        for value in entries {
            let resolved = match resolve_skill_value(value, root, home) {
                Ok(p) => p,
                Err(reason) => {
                    warnings.push(format!(
                        "{surface} skill '{value}' ignored — {reason}"
                    ));
                    continue;
                }
            };
            if !resolved.exists() {
                warnings.push(format!(
                    "{surface} skill '{value}' ignored — missing or unreadable at {}",
                    resolved.display()
                ));
                continue;
            }
            if !seen.insert(resolved.clone()) {
                continue; // de-duped against an earlier surface
            }
            args.push("--skill".to_string());
            args.push(resolved.display().to_string());
        }
    }

    if declarations.exclusive && !args.iter().any(|a| a == "--skill") {
        warnings.push(
            "skills-mode is exclusive but no resolvable skill is declared — the session runs with NO skills (exclusive mode is opt-in narrowing; declare a skill or drop the mode for the full discovery set)"
                .to_string(),
        );
    }

    (args, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("knot-skill-assign-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, rel: &str) -> PathBuf {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, "---\ndescription: test skill\n---\nbody\n").unwrap();
        p
    }

    fn decl(entries: &[&str], exclusive: bool) -> SkillDeclarations {
        SkillDeclarations {
            rig_entries: entries.iter().map(|s| s.to_string()).collect(),
            exclusive,
        }
    }

    // ── path resolution rules ────────────────────────────────────────────

    #[test]
    fn relative_resolves_against_root() {
        let root = temp_dir("rel");
        let p = resolve_skill_value("skills/a.md", &root, None).unwrap();
        assert_eq!(p, root.join("skills/a.md"));
    }

    #[test]
    fn absolute_used_as_is() {
        let root = temp_dir("abs");
        let p = resolve_skill_value("/opt/skills/a.md", &root, None).unwrap();
        assert_eq!(p, PathBuf::from("/opt/skills/a.md"));
    }

    #[test]
    fn home_expansion() {
        let home = temp_dir("home");
        let p =
            resolve_skill_value("~/sk/a.md", Path::new("/rig"), Some(&home)).unwrap();
        assert_eq!(p, home.join("sk/a.md"));
    }

    #[test]
    fn home_expansion_without_home_errors() {
        let e = resolve_skill_value("~/sk/a.md", Path::new("/rig"), None).unwrap_err();
        assert!(e.contains("$HOME"));
    }

    #[test]
    fn file_url_converted() {
        let p =
            resolve_skill_value("file:///opt/x/a.md", Path::new("/rig"), None).unwrap();
        assert_eq!(p, PathBuf::from("/opt/x/a.md"));
    }

    #[test]
    fn http_refused() {
        for v in ["http://x/a.md", "https://x/a.md"] {
            assert!(resolve_skill_value(v, Path::new("/rig"), None).is_err());
        }
    }

    // ── composition ──────────────────────────────────────────────────────

    #[test]
    fn empty_declarations_pass_no_flags() {
        let root = temp_dir("empty");
        let (args, warnings) = compose_skill_cli_args(
            &SkillDeclarations::default(),
            &[],
            &[],
            &root,
            &root,
            None,
        );
        assert!(args.is_empty(), "empty declaration = full discovery, unchanged");
        assert!(warnings.is_empty());
    }

    #[test]
    fn additive_union_in_surface_order() {
        let root = temp_dir("union");
        let a = touch(&root, "a.md");
        let b = touch(&root, "b.md");
        let c = touch(&root, "c.md");
        let (args, warnings) = compose_skill_cli_args(
            &decl(&["a.md"], false),
            &[b.display().to_string()],
            &[c.display().to_string()],
            &root,
            &root,
            None,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            args,
            vec![
                "--skill".to_string(),
                a.display().to_string(),
                "--skill".to_string(),
                b.display().to_string(),
                "--skill".to_string(),
                c.display().to_string(),
            ]
        );
        assert!(!args.contains(&"--no-skills".to_string()));
    }

    #[test]
    fn dedup_on_resolved_path_first_wins() {
        let root = temp_dir("dedup");
        let a = touch(&root, "a.md");
        let (args, warnings) = compose_skill_cli_args(
            &decl(&["a.md", "./a.md"], false),
            &[a.display().to_string()],
            &[],
            &root,
            &root,
            None,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            args,
            vec!["--skill".to_string(), a.display().to_string()],
            "three declarations of the same resolved path collapse to one"
        );
    }

    #[test]
    fn missing_skill_warns_and_skips_the_rest_continue() {
        let root = temp_dir("missing");
        let a = touch(&root, "a.md");
        let (args, warnings) = compose_skill_cli_args(
            &decl(&["gone.md", "a.md"], false),
            &[],
            &[],
            &root,
            &root,
            None,
        );
        assert_eq!(warnings.len(), 1, "one visible warning: {warnings:?}");
        assert!(warnings[0].contains("gone.md"));
        assert_eq!(
            args,
            vec!["--skill".to_string(), a.display().to_string()],
            "the resolvable remainder still loads"
        );
    }

    #[test]
    fn skill_directory_is_accepted() {
        let root = temp_dir("dir-skill");
        let d = root.join("my-skill");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), "---\ndescription: d\n---\n").unwrap();
        let (args, warnings) = compose_skill_cli_args(
            &decl(&["my-skill"], false),
            &[],
            &[],
            &root,
            &root,
            None,
        );
        assert!(warnings.is_empty());
        assert_eq!(args, vec!["--skill".to_string(), d.display().to_string()]);
    }

    #[test]
    fn exclusive_narrows_to_declared_set() {
        let root = temp_dir("exclusive");
        let a = touch(&root, "a.md");
        let (args, warnings) = compose_skill_cli_args(
            &decl(&["a.md"], true),
            &[],
            &[],
            &root,
            &root,
            None,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            args,
            vec![
                "--no-skills".to_string(),
                "--skill".to_string(),
                a.display().to_string(),
            ],
            "--no-skills comes first; explicit skills load anyway"
        );
    }

    #[test]
    fn exclusive_with_nothing_declared_warns_narrowing_to_nothing() {
        let root = temp_dir("exclusive-empty");
        let (args, warnings) = compose_skill_cli_args(
            &decl(&[], true),
            &[],
            &[],
            &root,
            &root,
            None,
        );
        assert_eq!(args, vec!["--no-skills".to_string()]);
        assert_eq!(warnings.len(), 1, "the degenerate case is never silent");
        assert!(warnings[0].contains("NO skills"));
    }

    #[test]
    fn config_entries_use_config_root_frontmatter_uses_effective_root() {
        let local = temp_dir("local-rig");
        let effective = temp_dir("effective-rig");
        let in_local = touch(&local, "proj-skill.md");
        let in_effective = touch(&effective, "wf-skill.md");
        let (args, warnings) = compose_skill_cli_args(
            &decl(&["proj-skill.md"], false),
            &["wf-skill.md".to_string()],
            &[],
            &local,
            &effective,
            None,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            args,
            vec![
                "--skill".to_string(),
                in_local.display().to_string(),
                "--skill".to_string(),
                in_effective.display().to_string(),
            ],
            "rig-config relative → local rig dir; frontmatter relative → effective rig dir"
        );
    }
}
