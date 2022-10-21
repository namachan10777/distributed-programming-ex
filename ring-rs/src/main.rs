use anyhow::{bail, Context};
use clap::Parser;
use proto::{
    join_response, ring_client::RingClient, CoordinateRequest, Empty, JoinRequest, JoinResponse,
    ListRequest, SetAddressRequest, SetAddressResponse,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{net::SocketAddr, process::exit};
use tokio::sync::{Mutex, RwLock};
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
    join_lock: Arc<Mutex<()>>,
    am_i_coordinator: Arc<RwLock<bool>>,
    connection: Arc<RwLock<Connection>>,
    ring_cache: Arc<RwLock<Vec<String>>>,
    notify_recieved_list_rpc: tokio::sync::mpsc::Sender<CooridnatorEvent>,
}

impl Ring {
    async fn join_impl_no_rollback<'a>(&self, new_prev: &str) -> anyhow::Result<()> {
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
        Ok(())
    }

    async fn join_impl(&self, new_prev: &str) -> anyhow::Result<String> {
        let _ = self.join_lock.lock().await;
        let old_prev = self.connection.read().await.prev;
        match self.join_impl_no_rollback(new_prev).await {
            Ok(()) => Ok(old_prev.to_string()),
            Err(e) => {
                self.connection.write().await.prev = old_prev;
                Err(e)
            }
        }
    }

    async fn relay_list_rpc_in_background(&self, mut addresses: Vec<String>) {
        let connection = self.connection.clone();
        let member_cache = self.ring_cache.read().await.clone();
        let me = self.me;
        tokio::spawn(async move {
            if let Ok(mut client) =
                RingClient::connect(format!("http://{}", connection.read().await.prev)).await
            {
                addresses.push(me.to_string());
                if let Err(e) = client.list(ListRequest { addresses }).await {
                    warn!("{}.list responses {}", connection.read().await.prev, e);
                    Self::recovery_prev(connection, member_cache, me).await;
                }
            } else {
                Self::recovery_prev(connection, member_cache, me).await;
            }
        });
    }

    async fn relay_election_rpc_in_background(&self, mut addresses: Vec<String>) {
        let connection = self.connection.clone();
        let member_cache = self.ring_cache.read().await.clone();
        let me = self.me;
        tokio::spawn(async move {
            if let Ok(mut client) =
                RingClient::connect(format!("http://{}", connection.read().await.prev)).await
            {
                addresses.push(me.to_string());
                if let Err(e) = client.election(ListRequest { addresses }).await {
                    warn!("{}.list responses {}", connection.read().await.prev, e);
                    Self::recovery_prev(connection, member_cache, me).await;
                }
            } else {
                Self::recovery_prev(connection, member_cache, me).await;
            }
        });
    }

    async fn relay_coordinate_rpc_in_background(
        &self,
        addresses: Vec<String>,
        mut read: Vec<String>,
    ) {
        let connection = self.connection.clone();
        let member_cache = self.ring_cache.read().await.clone();
        let me = self.me;
        tokio::spawn(async move {
            match RingClient::connect(format!("http://{}", connection.read().await.prev)).await {
                Ok(mut client) => {
                    read.push(me.to_string());
                    if let Err(e) = client
                        .coordinate(CoordinateRequest { addresses, read })
                        .await
                    {
                        warn!(
                            "{}.coordinate responses {}",
                            connection.read().await.prev,
                            e
                        );
                        Self::recovery_prev(connection, member_cache, me).await;
                    }
                }
                Err(e) => {
                    warn!(
                        "{}.coordinate responses {}",
                        connection.read().await.prev,
                        e
                    );
                    Self::recovery_prev(connection, member_cache, me).await;
                }
            }
        });
    }

    async fn announce_in_background(&self, addresses: Vec<String>) {
        info!("members: {:?}", addresses);
        tokio::spawn(async move {
            for address in &addresses {
                match RingClient::connect(format!("http://{}", address)).await {
                    Ok(mut client) => {
                        if let Err(e) = client
                            .connection_announce(ListRequest {
                                addresses: addresses.clone(),
                            })
                            .await
                        {
                            warn!("{}.connection_announce responses {}", address, e);
                        }
                    }
                    Err(e) => {
                        warn!("failde to connect {} due to {}", address, e);
                    }
                }
            }
        });
    }

    async fn set_next_wrapper(me: SocketAddr, dest: SocketAddr) -> anyhow::Result<()> {
        let mut client = RingClient::connect(format!("http://{}", dest)).await?;
        let response = client
            .set_next(SetAddressRequest {
                address: me.to_string(),
            })
            .await?;
        if let Some(err_msg) = response.into_inner().err_msg {
            bail!("{}", err_msg);
        }
        Ok(())
    }

    async fn set_prev_wrapper(me: SocketAddr, dest: SocketAddr) -> anyhow::Result<()> {
        let mut client = RingClient::connect(format!("http://{}", dest)).await?;
        let response = client
            .set_prev(SetAddressRequest {
                address: me.to_string(),
            })
            .await?;
        if let Some(err_msg) = response.into_inner().err_msg {
            bail!("{}", err_msg);
        }
        Ok(())
    }

    async fn recovery_prev(
        connection: Arc<RwLock<Connection>>,
        member_cache: Vec<String>,
        me: SocketAddr,
    ) {
        info!("recovery prev");
        if let Some(my_idx_in_member_cache) =
            member_cache.iter().enumerate().find_map(|(idx, address)| {
                if address == &me.to_string() {
                    Some(idx)
                } else {
                    None
                }
            })
        {
            for i in 0..member_cache.len() {
                let prev_idx = (my_idx_in_member_cache + i + 1) % member_cache.len();
                match member_cache[prev_idx].parse() {
                    Ok(prev) => {
                        if let Err(e) = Self::set_next_wrapper(me, prev).await {
                            info!("node {} doesn't working", e);
                        } else {
                            connection.write().await.prev = prev;
                            break;
                        }
                    }
                    Err(e) => info!(
                        "address {} is invalid due to {}",
                        &member_cache[prev_idx], e
                    ),
                }
            }
        } else {
            error!("my name does'nt found in ring cache");
        }
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
        let addresses = request.into_inner().addresses;

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
            self.relay_list_rpc_in_background(addresses).await;
        } else {
            self.announce_in_background(addresses).await;
        }
        Ok(Response::new(Empty {}))
    }

    async fn coordinate(
        &self,
        request: Request<CoordinateRequest>,
    ) -> Result<Response<Empty>, Status> {
        let CoordinateRequest { addresses, read } = request.into_inner();
        *self.am_i_coordinator.write().await =
            check_am_i_coordinator(&addresses, &self.me.to_string());
        if !read.contains(&self.me.to_string()) {
            self.relay_coordinate_rpc_in_background(addresses, read)
                .await;
        }

        Ok(Response::new(Empty {}))
    }
    async fn election(&self, request: Request<ListRequest>) -> Result<Response<Empty>, Status> {
        let addresses = request.into_inner().addresses;
        if !addresses.contains(&self.me.to_string()) {
            // TODO: Error handling
            self.relay_election_rpc_in_background(addresses).await;
        } else {
            self.relay_coordinate_rpc_in_background(addresses, Vec::new())
                .await;
        }
        Ok(Response::new(Empty {}))
    }

    async fn connection_announce(
        &self,
        request: Request<ListRequest>,
    ) -> Result<Response<Empty>, Status> {
        let addresses = request.into_inner().addresses;
        info!("announced connection: {:?}", addresses);
        *self.ring_cache.write().await = addresses;
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
        join_lock: Arc::new(Mutex::new(())),
        ring_cache: Arc::new(RwLock::new(Vec::new())),
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
                let prev_response = Ring::set_prev_wrapper(next, prev).await;
                let next_response = Ring::set_next_wrapper(prev, next).await;
                match (prev_response, next_response) {
                    (Ok(_), Ok(_)) => {}
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
