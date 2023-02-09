use std::{net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use clap::Parser;
use raft::{server::types::Log, Error};
use tokio::fs;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Opts {
    addr: SocketAddr,
    #[clap(long, default_value = "1000ms")]
    heatbeat: ClapDuration,
    election_timeout: Option<ClapDuration>,
    #[clap(short, long)]
    servers: PathBuf,
}

#[derive(Clone)]
struct ClapDuration(Duration);

impl FromStr for ClapDuration {
    type Err = humantime::DurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        humantime::parse_duration(s).map(Self)
    }
}

struct LogConsumer;

#[async_trait::async_trait]
impl raft::LogConsumer for LogConsumer {
    type Target = String;
    async fn recv(&self, _log: Log<Self::Target>) -> Result<(), Error> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::prelude::*;
    let opts = Opts::parse();
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let heatbeat = opts.heatbeat.0;
    let election_timeout = opts.election_timeout.map(|d| d.0).unwrap_or(heatbeat * 2);
    let servers = fs::read_to_string(&opts.servers).await?;
    let servers = serde_json::from_str(&servers)?;

    let raft = raft::server::Server::from_local_json(
        opts.addr,
        format!("{}.json", opts.addr),
        election_timeout,
        heatbeat,
        servers,
        LogConsumer,
    )
    .await?;

    info!(
        addr = opts.addr.to_string(),
        heatbeat = humantime::format_duration(heatbeat).to_string(),
        election_timeout = humantime::format_duration(election_timeout).to_string(),
        "prepared raft server"
    );

    let service = raft::server::proto::raft_server::RaftServer::new(raft);
    tonic::transport::Server::builder()
        .add_service(service)
        .serve(opts.addr)
        .await?;
    Ok(())
}
