use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use clap::Parser;
use raft::{
    append_entries, get_log, launch, post_log, request_vote, AppendEntriesRequest,
    RequestVoteRequest, State,
};
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
    let timeout = Duration::from_millis(1000);
    let heatbeat = Duration::from_millis(300);
    launch(state.clone(), timeout, heatbeat).await;
    let state = warp::any().map(move || state.clone());
    let append_entries = warp::path!("api" / "log")
        .and(state.clone())
        .and(warp::put())
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
    let request_vote = warp::path!("api" / "vote")
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
    let post_log = warp::path!("log")
        .and(state.clone())
        .and(warp::post())
        .and(warp::body::json::<Vec<String>>())
        .then(move |state, logs| async move {
            match post_log(state, logs, timeout).await {
                Ok(res) => warp::reply::with_status(warp::reply::json(&res), StatusCode::OK),
                Err(e) => warp::reply::with_status(
                    warp::reply::json(&e.to_string()),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        });
    let get_log = warp::path!("log")
        .and(state.clone())
        .and(warp::get())
        .then(|state| async move { warp::reply::json(&get_log(state).await) });
    warp::serve(append_entries.or(request_vote).or(post_log).or(get_log))
        .run(addr)
        .await;
    Ok(())
}
