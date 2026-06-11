use knot::AppConfig;

fn print_version() {
    println!("knot {}", env!("CARGO_PKG_VERSION"));
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        print_version();
        return Ok(());
    }
    knot::start_server(AppConfig::default_config()).await?;
    Ok(())
}
