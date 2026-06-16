use knot::AppConfig;
use knot::domain::rig_discovery::{discover_rigs, RigDiscovery};

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

    // share <rig-name> — not yet implemented (Phase 4)
    if args.first().map(|a| a.as_str()) == Some("share") {
        eprintln!("Share command is not yet implemented.");
        eprintln!("Usage: knot share <rig-name>");
        std::process::exit(1);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| {
        eprintln!("Error: cannot determine current working directory");
        std::process::exit(1);
    });

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
