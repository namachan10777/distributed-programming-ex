use anyhow::Context;
use clap::Parser;
use proto::{
    join_response, ring_client::RingClient, CoordinateRequest, Empty, JoinRequest, JoinResponse,
    ListRequest, SetAddressRequest, SetAddressResponse,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;

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

struct Ring {
    _me: SocketAddr,
    connection: Arc<RwLock<Connection>>,
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

    async fn list(&self, _request: Request<ListRequest>) -> Result<Response<Empty>, Status> {
        unimplemented!()
    }
    async fn coordinate(
        &self,
        _request: Request<CoordinateRequest>,
    ) -> Result<Response<Empty>, Status> {
        unimplemented!()
    }
    async fn election(&self, _request: Request<ListRequest>) -> Result<Response<Empty>, Status> {
        unimplemented!()
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
    let ring_service = Ring {
        _me: opts.addr,
        connection: connection.clone(),
    };

    let server_handler = tokio::spawn(
        Server::builder()
            .add_service(proto::ring_server::RingServer::new(ring_service))
            .serve(opts.addr),
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

    server_handler.await??;

    Ok(())
}
