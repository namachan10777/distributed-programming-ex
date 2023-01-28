use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing::info;

#[derive(Parser)]
struct Opts {}

#[tokio::main]
async fn main() {
    use tracing_subscriber::prelude::*;
    let _opts = Opts::parse();
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();
    info!("Hello World!");
}
