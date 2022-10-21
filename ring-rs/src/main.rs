use anyhow::Context;
use clap::Parser;
use proto::{
    join_response, ring_client::RingClient, CoordinateRequest, Empty, JoinRequest, JoinResponse,
    ListRequest, SetAddressRequest, SetAddressResponse,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{net::SocketAddr, process::exit};
use tokio::sync::RwLock;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{error, info, warn};

mod proto {
    tonic::include_proto!("ring");
}

#[derive(clap::Parser)]
struct Opts {
    addr: SocketAddr,
    next: Option<SocketAddr>,
}

struct Connection {
    prev: SocketAddr,
    next: SocketAddr,
}

enum CooridnatorEvent {
    HealthCheckRecieved,
}

struct Ring {
    me: SocketAddr,
    am_i_coordinator: Arc<RwLock<bool>>,
    connection: Arc<RwLock<Connection>>,
    notify_recieved_list_rpc: tokio::sync::mpsc::Sender<CooridnatorEvent>,
}

impl Ring {
    async fn join_impl(&self, new_prev: &str) -> anyhow::Result<String> {
        let new_prev = new_prev.parse()?;
        let old_prev = {
            let mut connection = self.connection.write().await;
            let old_prev = connection.prev;
            info!("prev: {} -> {}", old_prev, new_prev);
            connection.prev = new_prev;
            old_prev
        };
        let mut client = RingClient::connect(format!("http://{}", old_prev)).await?;
        let set_next_response = client
            .set_next(SetAddressRequest {
                address: new_prev.to_string(),
            })
            .await?;
        if let Some(err_msg) = set_next_response.into_inner().err_msg {
            return Err(anyhow::anyhow!("{}", err_msg));
        }
        Ok(old_prev.to_string())
    }
}

fn check_am_i_coordinator(addresses: &[String], me: &str) -> bool {
    addresses.iter().all(|address| address.as_str() >= me)
}

#[tonic::async_trait]
impl proto::ring_server::Ring for Ring {
    async fn set_next(
        &self,
        request: Request<SetAddressRequest>,
    ) -> Result<Response<SetAddressResponse>, Status> {
        match request.into_inner().address.parse() {
            Ok(next_addr) => {
                {
                    let mut connection = self.connection.write().await;
                    info!("next: {} -> {}", connection.next, next_addr);
                    connection.next = next_addr;
                }
                Ok(Response::new(SetAddressResponse { err_msg: None }))
            }
            Err(e) => Ok(Response::new(SetAddressResponse {
                err_msg: Some(e.to_string()),
            })),
        }
    }

    async fn set_prev(
        &self,
        request: Request<SetAddressRequest>,
    ) -> Result<Response<SetAddressResponse>, Status> {
        match request.into_inner().address.parse() {
            Ok(prev_addr) => {
                {
                    let mut connection = self.connection.write().await;
                    info!("prev: {} -> {}", connection.prev, prev_addr);
                    connection.prev = prev_addr;
                }
                Ok(Response::new(SetAddressResponse { err_msg: None }))
            }
            Err(e) => Ok(Response::new(SetAddressResponse {
                err_msg: Some(e.to_string()),
            })),
        }
    }

    async fn join(&self, request: Request<JoinRequest>) -> Result<Response<JoinResponse>, Status> {
        let response = self
            .join_impl(&request.into_inner().prev)
            .await
            .map(|old_prev| JoinResponse {
                result: Some(join_response::Result::Prev(old_prev)),
            })
            .unwrap_or_else(|e| JoinResponse {
                result: Some(join_response::Result::ErrMsg(e.to_string())),
            });
        Ok(Response::new(response))
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<Empty>, Status> {
        let mut addresses = request.into_inner().addresses;

        if let Err(e) = self
            .notify_recieved_list_rpc
            .send(CooridnatorEvent::HealthCheckRecieved)
            .await
        {
            error!("Cannot notify coordinator event due to {}", e);
            exit(1);
        }
        if !addresses.contains(&self.me.to_string()) {
            // TODO: Error handling
            // TODO: request to next in background
            if let Ok(mut client) =
                RingClient::connect(format!("http://{}", self.connection.read().await.prev)).await
            {
                addresses.push(self.me.to_string());
                if let Err(e) = client.list(ListRequest { addresses }).await {
                    warn!("{}.list responses {}", self.connection.read().await.prev, e);
                }
            }
        } else {
            info!("members: {:?}", addresses);
        }
        Ok(Response::new(Empty {}))
    }
    async fn coordinate(
        &self,
        request: Request<CoordinateRequest>,
    ) -> Result<Response<Empty>, Status> {
        let CoordinateRequest {
            addresses,
            mut read,
        } = request.into_inner();
        *self.am_i_coordinator.write().await =
            check_am_i_coordinator(&addresses, &self.me.to_string());
        if !read.contains(&self.me.to_string()) {
            match RingClient::connect(format!("http://{}", self.connection.read().await.prev)).await
            {
                Ok(mut client) => {
                    read.push(self.me.to_string());
                    if let Err(e) = client
                        .coordinate(CoordinateRequest { addresses, read })
                        .await
                    {
                        warn!(
                            "{}.coordinate responses {}",
                            self.connection.read().await.prev,
                            e
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "{}.coordinate responses {}",
                        self.connection.read().await.prev,
                        e
                    );
                }
            }
        }

        Ok(Response::new(Empty {}))
    }
    async fn election(&self, request: Request<ListRequest>) -> Result<Response<Empty>, Status> {
        let mut addresses = request.into_inner().addresses;
        if !addresses.contains(&self.me.to_string()) {
            // TODO: Error handling
            // TODO: request to next in background
            match RingClient::connect(format!("http://{}", self.connection.read().await.prev)).await
            {
                Ok(mut client) => {
                    addresses.push(self.me.to_string());
                    if let Err(e) = client.election(ListRequest { addresses }).await {
                        warn!("{}.list responses {}", self.connection.read().await.prev, e);
                    }
                }
                Err(e) => {
                    warn!(
                        "cannot create client for {} due to {}",
                        self.connection.read().await.prev,
                        e
                    );
                }
            }
        } else {
            match RingClient::connect(format!("http://{}", self.connection.read().await.prev)).await
            {
                Ok(mut client) => {
                    if let Err(e) = client
                        .coordinate(CoordinateRequest {
                            addresses,
                            read: vec![self.me.to_string()],
                        })
                        .await
                    {
                        warn!(
                            "{}.coordinate responses {}",
                            self.connection.read().await.prev,
                            e
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "cannot create client for {} due to {}",
                        self.connection.read().await.prev,
                        e
                    );
                }
            }
        }
        Ok(Response::new(Empty {}))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    tracing_subscriber::fmt::init();

    let connection = Arc::new(RwLock::new(Connection {
        next: opts.addr,
        prev: opts.addr,
    }));

    let (notify_recieved_list_rpc, mut recieved_list_rpc) = tokio::sync::mpsc::channel(1024);
    let am_i_coordinator = Arc::new(RwLock::new(false));

    let ring_service = Ring {
        me: opts.addr,
        am_i_coordinator: am_i_coordinator.clone(),
        connection: connection.clone(),
        notify_recieved_list_rpc,
    };

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    let connection_for_shutdown_handler = connection.clone();

    let server_handler = tokio::spawn(
        Server::builder()
            .add_service(proto::ring_server::RingServer::new(ring_service))
            .serve_with_shutdown(opts.addr, async move {
                tokio::select! {
                    _ = sigterm.recv() => (),
                    _ = sigint.recv() => (),
                }
                info!("start graceful shutdown");
                let Connection { prev, next } = *connection_for_shutdown_handler.read().await;
                let prev_client = RingClient::connect(format!("http://{}", prev)).await;
                let next_client = RingClient::connect(format!("http://{}", next)).await;
                match (prev_client, next_client) {
                    (Ok(mut prev_client), Ok(mut next_client)) => {
                        if let Err(e) = prev_client
                            .set_next(SetAddressRequest {
                                address: next.to_string(),
                            })
                            .await
                        {
                            warn!("{}.set_next responses {}", prev, e);
                        }
                        if let Err(e) = next_client
                            .set_prev(SetAddressRequest {
                                address: prev.to_string(),
                            })
                            .await
                        {
                            warn!("{}.set_prev responses {}", next, e);
                        }
                    }
                    (Err(e), _) => {
                        warn!("connect to {} failed due to {}", prev, e)
                    }
                    (_, Err(e)) => {
                        warn!("connect to {} failed due to {}", next, e)
                    }
                }
            }),
    );

    tokio::time::sleep(Duration::from_millis(100)).await;

    if let Some(next) = opts.next {
        let mut client = RingClient::connect(format!("http://{}", next)).await?;
        let response = client
            .join(JoinRequest {
                prev: opts.addr.to_string(),
            })
            .await?;
        match response
            .into_inner()
            .result
            .with_context(|| "no contents from join rpc")?
        {
            join_response::Result::ErrMsg(e) => anyhow::bail!("{}", e),
            join_response::Result::Prev(prev) => {
                let mut connection = connection.write().await;
                connection.next = next;
                connection.prev = prev.parse()?;
                info!("next: {}", connection.next);
                info!("prev: {}", connection.prev);
            }
        }
    }

    let connection_for_watcher = connection.clone();

    let coordinator_watcher = tokio::spawn(async move {
        let connection = connection_for_watcher;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {
                    info!("coordinator has gone!");
                    match RingClient::connect(format!("http://{}", connection.read().await.prev)).await {
                        Ok(mut client) => {
                            if let Err(e) = client.election(ListRequest {
                                addresses: vec![opts.addr.to_string()],
                            }).await {
                                warn!("{}.election responses {}", connection.read().await.prev, e);
                            }
                        },
                        Err(e) => {
                            warn!("connect to {} failed due to {}", connection.read().await.prev, e);
                        }
                    }
                },
                _ = recieved_list_rpc.recv() => {}
            }
        }
    });

    let coordinate = tokio::spawn(async move {
        loop {
            let now = Instant::now();
            if *am_i_coordinator.read().await {
                info!("i am coordinator");
                match RingClient::connect(format!("http://{}", connection.read().await.prev)).await
                {
                    Ok(mut client) => {
                        if let Err(e) = client
                            .list(ListRequest {
                                addresses: vec![opts.addr.to_string()],
                            })
                            .await
                        {
                            warn!("{}.list responses {}", connection.read().await.prev, e);
                        }
                    }
                    Err(e) => {
                        warn!(
                            "connect to {} failed due to {}",
                            connection.read().await.prev,
                            e
                        );
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2) - now.elapsed()).await;
        }
    });

    tokio::select! {
        r = server_handler => {
            match r {
                Ok(Ok(_)) => info!("exit"),
                Ok(Err(e)) => error!("{}", e),
                Err(e) => error!("{}", e),
            }
        },
        Err(e) = coordinator_watcher => {
            error!("{}", e);
        },
        Err(e) = coordinate => {
            error!("{}", e);
        },
    }

    Ok(())
}
