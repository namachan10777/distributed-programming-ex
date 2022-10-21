use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, net::SocketAddr};
use tokio::net;
use tracing::warn;

#[derive(clap::Parser)]
struct Opts {
    addr: SocketAddr,
}

#[derive(Deserialize)]
enum Request {
    Query,
    Register(String),
}
#[derive(Serialize)]
struct Response<'a> {
    addresses: &'a HashSet<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let opts = Opts::parse();
    let socket = net::UdpSocket::bind(opts.addr).await?;
    let mut buf = [0u8; 2048];
    let mut addresses = HashSet::new();
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((_, addr)) => match rmp_serde::from_slice(&buf) {
                Ok(req) => match req {
                    Request::Query => {
                        let res = rmp_serde::to_vec(&Response {
                            addresses: &addresses,
                        })
                        .unwrap();
                        if let Err(e) = socket.send_to(&res, addr).await {
                            warn!("{} to {}", e, addr);
                        }
                    }
                    Request::Register(addr) => {
                        addresses.insert(addr);
                    }
                },
                Err(e) => {
                    warn!("{}", e);
                }
            },
            Err(e) => {
                warn!("{}", e);
            }
        }
    }
}
