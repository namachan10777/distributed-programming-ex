use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use clap::Parser;
use raft::{append_entries, launch, request_vote, AppendEntriesRequest, RequestVoteRequest, State};
use reqwest::StatusCode;
use tokio::fs;
use tracing_subscriber::EnvFilter;
use warp::Filter;

#[derive(Parser)]
struct Opts {
    addr: SocketAddr,
    #[clap(short, long, default_value = "servers.json")]
    servers: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::prelude::*;
    let _opts = Opts::parse();
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let opts = Opts::parse();
    let addr = opts.addr;
    let servers = fs::read_to_string(opts.servers).await?;
    let servers = serde_json::from_str(&servers)?;
    let state = Arc::new(State::from_file(format!("{addr}.json"), addr, servers).await?);
    launch(
        state.clone(),
        Duration::from_millis(1000),
        Duration::from_millis(300),
    )
    .await;
    let state = warp::any().map(move || state.clone());
    let append_entries = warp::path!("append_entries")
        .and(state.clone())
        .and(warp::post())
        .and(warp::body::json::<AppendEntriesRequest>())
        .then(|state, req| async move {
            match append_entries(state, req).await {
                Ok(ok) => warp::reply::with_status(warp::reply::json(&ok), StatusCode::ACCEPTED),
                Err(e) => warp::reply::with_status(
                    warp::reply::json(&e.to_string()),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        });
    let request_vote = warp::path!("request_vote")
        .and(state.clone())
        .and(warp::post())
        .and(warp::body::json::<RequestVoteRequest>())
        .then(|state, req| async move {
            match request_vote(state, req).await {
                Ok(ok) => warp::reply::with_status(warp::reply::json(&ok), StatusCode::ACCEPTED),
                Err(e) => warp::reply::with_status(
                    warp::reply::json(&e.to_string()),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        });
    warp::serve(append_entries.or(request_vote)).run(addr).await;
    Ok(())
}
