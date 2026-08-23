use knot::AppConfig;
use knot::domain::rig_discovery::{discover_rigs, RigDiscovery};

use std::io::Write;
use std::path::Path;
use zip::write::FileOptions;
use zip::ZipWriter;

fn print_version() {
    println!("knot {}", env!("CARGO_PKG_VERSION"));
}

fn print_usage() {
    println!(
        "\
Usage: knot [OPTIONS] [COMMAND] [rig-name]

A local agent orchestration service.

Commands:
  share <rig-name>  Package rig looms and profiles into a .zip
  step              Process exactly one queued event, then exit

Options:
  -V, --version     Print version
  -h, --help        Print this help

step options (only valid after `step`):
  --rig <rig-name>  Target a specific rig (same `./<name>` resolution
                    as the service positional)
  --event <spec>    Target a specific queued event: exact event id
                    (`.json` optional), unique id prefix, or strand
                    filename

Arguments:
  <rig-name>        Start with the named rig directory
                    (e.g. `knot dev-rig` uses `./dev-rig/`)

If no rig-name is given, Knot auto-discovers `*-rig` directories
in the current working directory:
  - Zero matches  → creates `rig/` and uses it
  - One match     → uses that rig
  - Multiple      → error (specify one explicitly)

`step` rig discovery is stricter: zero matches is an error (no
implicit `rig/` creation) and multiple matches is an error.

Exit codes (all commands):
  0  success (for `step`: an event was processed, or the queue was
     empty)
  1  error (unknown event, multiple rigs, processing failure, ...)
"  
    );
}

/// A fully parsed CLI invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    /// Start the service (the default command).
    ///
    /// `rig` is the explicit positional rig name, if given.
    Service { rig: Option<String> },
    /// Package a rig's looms and profiles into a `.zip`.
    Share { rig: String },
    /// Process exactly one queued event, then exit.
    ///
    /// `rig` targets a specific rig (`--rig`); `event` targets a
    /// specific queued event (`--event`). Without `event`, the FIFO
    /// head is processed; an empty queue is a no-op (exit 0).
    Step {
        rig: Option<String>,
        event: Option<String>,
    },
    /// Print usage (`--help` / `-h`).
    Help,
    /// Print version (`--version` / `-V`).
    Version,
}

/// Parse CLI arguments into a [`CliCommand`].
///
/// Pure function — no env access, no I/O, no process exit — so the
/// grammar is unit-testable. Errors are human-readable strings.
///
/// Grammar:
/// - `--version` / `-V` / `--help` / `-h` are global: the first
///   occurrence wins, anywhere in the argument list.
/// - The first positional is a command (`share` or `step`) or the
///   service's rig name.
/// - `--rig <v>` and `--event <v>` are only valid **after** `step`;
///   flag order between them is free; missing values are errors;
///   duplicates are errors.
/// - `share` takes exactly one positional (the rig name).
/// - Any other argument starting with `-` is an unknown-flag error.
fn parse_args(args: &[String]) -> Result<CliCommand, String> {
    let mut command: Option<&'static str> = None;
    let mut service_rig: Option<String> = None;
    let mut share_rig: Option<String> = None;
    let mut step_rig: Option<String> = None;
    let mut step_event: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--version" | "-V" => return Ok(CliCommand::Version),
            "--help" | "-h" => return Ok(CliCommand::Help),
            "--rig" => {
                if command != Some("step") {
                    return Err(
                        "unknown flag '--rig' (only valid with `step`)".into(),
                    );
                }
                if step_rig.is_some() {
                    return Err("--rig given more than once".into());
                }
                i += 1;
                step_rig = Some(
                    args.get(i)
                        .ok_or_else(|| "missing value for --rig".to_string())?
                        .clone(),
                );
            }
            "--event" => {
                if command != Some("step") {
                    return Err(
                        "unknown flag '--event' (only valid with `step`)".into(),
                    );
                }
                if step_event.is_some() {
                    return Err("--event given more than once".into());
                }
                i += 1;
                step_event = Some(
                    args.get(i)
                        .ok_or_else(|| "missing value for --event".to_string())?
                        .clone(),
                );
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return Err(format!("unknown flag '{arg}'"));
            }
            _ => match positionals_seen(command, service_rig.is_some(), share_rig.is_some()) {
                PosState::CommandSlot => match arg {
                    "share" => command = Some("share"),
                    "step" => command = Some("step"),
                    other => service_rig = Some(other.to_string()),
                },
                PosState::ShareRigSlot => share_rig = Some(arg.to_string()),
                PosState::AfterStep => {
                    return Err(format!(
                        "unexpected argument '{arg}' (use `--rig <rig-name>` for `step`)"
                    ));
                }
                PosState::Full => {
                    return Err(format!("unexpected argument '{arg}'"));
                }
            },
        }
        i += 1;
    }

    match command {
        Some("share") => share_rig
            .map(|rig| CliCommand::Share { rig })
            .ok_or_else(|| "share requires a rig name".to_string()),
        Some("step") => Ok(CliCommand::Step {
            rig: step_rig,
            event: step_event,
        }),
        _ => Ok(CliCommand::Service { rig: service_rig }),
    }
}

/// Which positional slot (if any) the next positional fills.
enum PosState {
    /// No command yet and no service rig: the positional is either a
    /// command name (`share`/`step`) or the service rig name.
    CommandSlot,
    /// `share` was seen and its rig name is still missing.
    ShareRigSlot,
    /// `step` was seen: no positionals are allowed (`--rig` only).
    AfterStep,
    /// The grammar is already complete: any further positional is an
    /// error.
    Full,
}

fn positionals_seen(
    command: Option<&'static str>,
    has_service_rig: bool,
    has_share_rig: bool,
) -> PosState {
    match command {
        Some("share") => {
            if has_share_rig {
                PosState::Full
            } else {
                PosState::ShareRigSlot
            }
        }
        Some("step") => PosState::AfterStep,
        None => {
            if has_service_rig {
                PosState::Full
            } else {
                PosState::CommandSlot
            }
        }
        Some(_) => PosState::Full,
    }
}

/// Package a rig's looms and profiles into a `.zip` archive.
///
/// Walks the rig directory, collects all `*-loom/` subdirectories
/// and `profiles/`, writes `<rig-name>.zip` in `output_dir`.
/// The rig directory holds reusable source only — runtime artifacts
/// (tie-offs, logs, state, queue) live in the project-level
/// `tie-offs/<rig-name>/` tree and are never part of the zip.
fn share_rig(output_dir: &Path, rig_path: &Path, rig_name: &str) {
    let zip_path = output_dir.join(format!("{}.zip", rig_name));

    let file = std::fs::File::create(&zip_path).unwrap_or_else(|e| {
        eprintln!("Error: cannot create '{}': {}", zip_path.display(), e);
        std::process::exit(1);
    });

    let mut writer = ZipWriter::new(file);
    let options = FileOptions::default();

    let rig_dir_name = rig_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| rig_name.to_string());

    // Collect all `*-loom/` subdirectories
    let looms: Vec<_> = match std::fs::read_dir(rig_path) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().is_dir()
                    && e.file_name()
                        .to_string_lossy()
                        .ends_with("-loom")
            })
            .map(|e| e.path())
            .collect(),
        Err(e) => {
            eprintln!("Error: cannot read rig directory: {}", e);
            std::process::exit(1);
        }
    };

    // Add each loom's files
    for loom_path in &looms {
        let loom_name = loom_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let entries = match std::fs::read_dir(loom_path) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "Error: cannot read '{}': {}",
                    loom_path.display(),
                    e
                );
                continue;
            }
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let zip_entry = format!(
                "{}/{}/{}",
                rig_dir_name, loom_name, file_name
            );

            let content = match std::fs::read(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Error: cannot read '{}': {}",
                        path.display(),
                        e
                    );
                    continue;
                }
            };

            if writer.start_file(&zip_entry, options).is_err() {
                eprintln!("Error: failed to write '{}' to zip", zip_entry);
                continue;
            }
            let _ = writer.write_all(&content);
        }
    }

    // Add profiles/ directory
    let profiles_path = rig_path.join("profiles");
    if profiles_path.is_dir() {
        let entries = match std::fs::read_dir(&profiles_path) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "Error: cannot read profiles directory: {}",
                    e
                );
                return;
            }
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let zip_entry = format!(
                "{}/profiles/{}",
                rig_dir_name, file_name
            );

            let content = match std::fs::read(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Error: cannot read '{}': {}",
                        path.display(),
                        e
                    );
                    continue;
                }
            };

            if writer.start_file(&zip_entry, options).is_err() {
                eprintln!("Error: failed to write '{}' to zip", zip_entry);
                continue;
            }
            let _ = writer.write_all(&content);
        }
    }

    if writer.finish().is_err() {
        eprintln!("Error: failed to finalize zip archive");
        std::process::exit(1);
    }

    println!("Packed {} into {}", rig_name, zip_path.display());
}

/// The current working directory, or exit with an error message.
fn current_dir_or_exit() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| {
        eprintln!("Error: cannot determine current working directory");
        std::process::exit(1);
    })
}

/// `share <rig-name>` — package the rig and exit the process.
fn share_command(rig: &str) -> ! {
    let cwd = current_dir_or_exit();
    let rig_path = cwd.join(rig);
    if !rig_path.is_dir() {
        eprintln!("Error: rig directory '{}' does not exist", rig_path.display());
        std::process::exit(1);
    }
    share_rig(&cwd, &rig_path, rig);
    std::process::exit(0);
}

/// Report multiple rigs found and exit (1).
fn multiple_rigs_error(paths: &[std::path::PathBuf], hint: &str) -> ! {
    let names: Vec<&str> = paths
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .collect();
    eprintln!("Error: multiple rigs found:\n  {}", names.join("\n  "));
    eprintln!("Specify one explicitly: {hint}");
    std::process::exit(1);
}

/// Service-mode rig resolution: auto-discovery applies — zero matches
/// falls through to the default (creates `rig/`), one match is used,
/// multiple matches is an error.
fn resolve_service_config(explicit: Option<&str>) -> AppConfig {
    let cwd = current_dir_or_exit();
    match discover_rigs(&cwd, explicit) {
        RigDiscovery::None => AppConfig::default_config(),
        RigDiscovery::Single(path) | RigDiscovery::Named(path) => {
            AppConfig::with_rig_dir(path)
        }
        RigDiscovery::Multiple(paths) => {
            multiple_rigs_error(&paths, "knot <rig-name>")
        }
    }
}

/// Step-mode rig resolution: stricter than the service — zero matches
/// is an error (no implicit `rig/` creation) and multiple matches is
/// an error.
fn resolve_step_config(explicit: Option<&str>) -> AppConfig {
    let cwd = current_dir_or_exit();
    match discover_rigs(&cwd, explicit) {
        RigDiscovery::None => {
            eprintln!("Error: no rigs found (step does not create `rig/`)");
            eprintln!("Run `knot --help` for usage.");
            std::process::exit(1);
        }
        RigDiscovery::Single(path) | RigDiscovery::Named(path) => {
            AppConfig::with_rig_dir(path)
        }
        RigDiscovery::Multiple(paths) => {
            multiple_rigs_error(&paths, "knot step --rig <rig-name>")
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = parse_args(&args).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        eprintln!("Run `knot --help` for usage.");
        std::process::exit(1);
    });

    match command {
        CliCommand::Version => {
            print_version();
            Ok(())
        }
        CliCommand::Help => {
            print_usage();
            Ok(())
        }
        CliCommand::Share { rig } => share_command(&rig),
        CliCommand::Service { rig } => {
            let config = resolve_service_config(rig.as_deref());
            knot::start_knot(config).await
        }
        CliCommand::Step { rig, event } => {
            let config = resolve_step_config(rig.as_deref());
            knot::step_knot(config, event).await
        }
    }
}

// ── parse_args unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod parse_args_tests {
    use super::*;

    /// Build a `Vec<String>` from string literals.
    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    /// Parse and unwrap — panics with the error message on `Err`.
    fn ok(s: &[&str]) -> CliCommand {
        parse_args(&args(s))
            .unwrap_or_else(|e| panic!("expected Ok, got Err: {e}"))
    }

    /// Parse and assert the error message contains `needle`.
    fn err(s: &[&str], needle: &str) {
        let e = unwrap_err(s);
        assert!(
            e.contains(needle),
            "error should contain '{needle}', got: '{e}' for {s:?}"
        );
    }

    /// Parse and return the error message, panicking on `Ok`.
    fn unwrap_err(s: &[&str]) -> String {
        match parse_args(&args(s)) {
            Ok(cmd) => panic!("expected Err for {s:?}, got Ok({cmd:?})"),
            Err(e) => e,
        }
    }

    // ── service mode (default) ────────────────────────────────────

    #[test]
    fn no_args_is_service_without_rig() {
        assert_eq!(ok(&[]), CliCommand::Service { rig: None });
    }

    #[test]
    fn positional_is_service_rig() {
        assert_eq!(
            ok(&["dev-rig"]),
            CliCommand::Service {
                rig: Some("dev-rig".to_string())
            }
        );
    }

    #[test]
    fn two_positionals_in_service_mode_is_an_error() {
        err(&["a-rig", "b-rig"], "unexpected argument");
    }

    #[test]
    fn rig_flag_in_service_mode_is_rejected() {
        err(&["--rig", "a-rig"], "unknown flag '--rig'");
        err(&["a-rig", "--rig", "b-rig"], "unknown flag '--rig'");
        err(&["--event", "x"], "unknown flag '--event'");
    }

    #[test]
    fn rig_flag_before_step_is_rejected() {
        err(&["--rig", "a-rig", "step"], "unknown flag '--rig'");
    }

    #[test]
    fn unknown_flag_is_an_error() {
        err(&["--unknown-flag"], "unknown flag '--unknown-flag'");
        err(&["-x"], "unknown flag '-x'");
        err(&["step", "--bogus"], "unknown flag '--bogus'");
    }

    // ── share (unchanged grammar) ─────────────────────────────────

    #[test]
    fn share_with_rig_name() {
        assert_eq!(ok(&["share", "dev-rig"]), CliCommand::Share { rig: "dev-rig".to_string() });
    }

    #[test]
    fn share_without_rig_name_is_an_error() {
        err(&["share"], "share requires a rig name");
    }

    #[test]
    fn share_with_extra_positional_is_an_error() {
        err(&["share", "a", "b"], "unexpected argument");
    }

    // ── step ──────────────────────────────────────────────────────

    #[test]
    fn step_alone() {
        assert_eq!(ok(&["step"]), CliCommand::Step { rig: None, event: None });
    }

    #[test]
    fn step_with_rig() {
        assert_eq!(
            ok(&["step", "--rig", "dev-rig"]),
            CliCommand::Step {
                rig: Some("dev-rig".to_string()),
                event: None
            }
        );
    }

    #[test]
    fn step_with_event() {
        assert_eq!(
            ok(&["step", "--event", "1234-abcd.json"]),
            CliCommand::Step {
                rig: None,
                event: Some("1234-abcd.json".to_string())
            }
        );
    }

    #[test]
    fn step_with_both_flags() {
        assert_eq!(
            ok(&["step", "--rig", "dev-rig", "--event", "x.md"]),
            CliCommand::Step {
                rig: Some("dev-rig".to_string()),
                event: Some("x.md".to_string())
            }
        );
    }

    #[test]
    fn step_flag_order_is_free() {
        let forward = ok(&["step", "--rig", "a", "--event", "b"]);
        let reversed = ok(&["step", "--event", "b", "--rig", "a"]);
        assert_eq!(forward, reversed);
        assert_eq!(
            forward,
            CliCommand::Step {
                rig: Some("a".to_string()),
                event: Some("b".to_string())
            }
        );
    }

    #[test]
    fn step_missing_flag_value_is_an_error() {
        err(&["step", "--rig"], "missing value for --rig");
        err(&["step", "--event"], "missing value for --event");
        err(&["step", "--rig", "a", "--event"], "missing value for --event");
    }

    #[test]
    fn step_duplicate_flag_is_an_error() {
        err(&["step", "--rig", "a", "--rig", "b"], "more than once");
        err(&["step", "--event", "a", "--event", "b"], "more than once");
    }

    #[test]
    fn step_positional_is_an_error() {
        let e = unwrap_err(&["step", "positional"]);
        assert!(
            e.contains("--rig"),
            "error should point to --rig, got: '{e}'"
        );
    }

    // ── global flags ──────────────────────────────────────────────

    #[test]
    fn version_flag_anywhere() {
        assert_eq!(ok(&["--version"]), CliCommand::Version);
        assert_eq!(ok(&["-V"]), CliCommand::Version);
        assert_eq!(ok(&["step", "--version"]), CliCommand::Version);
        assert_eq!(ok(&["myrig", "-V"]), CliCommand::Version);
    }

    #[test]
    fn help_flag_anywhere() {
        assert_eq!(ok(&["--help"]), CliCommand::Help);
        assert_eq!(ok(&["-h"]), CliCommand::Help);
        assert_eq!(ok(&["step", "--help"]), CliCommand::Help);
    }
}
