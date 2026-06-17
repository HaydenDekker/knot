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

Options:
  -V, --version     Print version
  -h, --help        Print this help

Arguments:
  <rig-name>        Start with the named rig directory
                    (e.g. `knot dev-rig` uses `./dev-rig/`)

If no rig-name is given, Knot auto-discovers `*-rig` directories
in the current working directory:
  - Zero matches  → creates `rig/` and uses it
  - One match     → uses that rig
  - Multiple      → error (specify one explicitly)
"  
    );
}

/// Package a rig's looms and profiles into a `.zip` archive.
///
/// Walks the rig directory, collects all `*-loom/` subdirectories
/// and `profiles/`, writes `<rig-name>.zip` in `output_dir`.
/// Excludes `tie-offs/`, `.rig-log`, and `.workspace-agent-config.yaml`
/// (derived state not needed by the recipient).
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

/// Parse CLI arguments and return the resolved `AppConfig`.
///
/// Exits the process on `--version`, `--help`, multiple rigs found,
/// or unsupported share command.
fn resolve_config() -> AppConfig {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // --version / -V
    if args.iter().any(|a| a == "--version" || a == "-V") {
        print_version();
        std::process::exit(0);
    }

    // --help / -h
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(0);
    }

    // Reject unknown flags (arguments starting with `--` that aren't known)
    let known_flags = ["--version", "-V", "--help", "-h"];
    for arg in &args {
        if arg.starts_with("--") && !known_flags.contains(&arg.as_str()) {
            eprintln!("Error: unknown flag '{}'", arg);
            eprintln!("Run `knot --help` for usage.");
            std::process::exit(1);
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| {
        eprintln!("Error: cannot determine current working directory");
        std::process::exit(1);
    });

    // share <rig-name>
    if args.first().map(|a| a.as_str()) == Some("share") {
        let rig_name = match args.get(1) {
            Some(name) => name.clone(),
            None => {
                eprintln!("Error: share requires a rig name");
                eprintln!("Usage: knot share <rig-name>");
                std::process::exit(1);
            }
        };
        let rig_path = cwd.join(&rig_name);
        if !rig_path.is_dir() {
            eprintln!("Error: rig directory '{}' does not exist", rig_path.display());
            std::process::exit(1);
        }
        share_rig(&cwd, &rig_path, &rig_name);
        std::process::exit(0);
    }

    let explicit_name = args.first().map(|s| s.as_str());
    let discovery = discover_rigs(&cwd, explicit_name);

    match discovery {
        RigDiscovery::None => {
            // No `*-rig` dirs found — fall through to default (creates `rig/`)
            AppConfig::default_config()
        }
        RigDiscovery::Single(path) => {
            AppConfig::with_rig_dir(path)
        }
        RigDiscovery::Named(path) => {
            AppConfig::with_rig_dir(path)
        }
        RigDiscovery::Multiple(paths) => {
            let names: Vec<&str> = paths
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect();
            eprintln!(
                "Error: multiple rigs found:\n  {}",
                names.join("\n  ")
            );
            eprintln!("Specify one explicitly: knot <rig-name>");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = resolve_config();
    knot::start_server(config).await?;
    Ok(())
}
