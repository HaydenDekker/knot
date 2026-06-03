use knot::AppConfig;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let _addr = knot::start_server(AppConfig::default_config()).await?;
    Ok(())
}
