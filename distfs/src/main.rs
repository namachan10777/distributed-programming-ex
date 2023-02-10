use clap::Parser;
use distfs::proto::fs::filesystem_server::FilesystemServer;
use fuse3::path::prelude::*;
use fuse3::MountOptions;
use std::{net::SocketAddr, num::NonZeroUsize, path::PathBuf};
use tonic::transport::{Server, Uri};
use tracing_subscriber::prelude::*;

#[derive(clap::Parser, Debug)]
struct Opts {
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Parser, Debug)]
enum SubCommand {
    Server {
        target: PathBuf,
        addr: SocketAddr,
    },
    Client {
        uri: Uri,
        mountpoint: PathBuf,
        replica: Option<NonZeroUsize>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();
    let opts = Opts::parse();

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    let mut mount_options = MountOptions::default();
    mount_options
        .uid(uid)
        .gid(gid)
        .read_only(false)
        .force_readdir_plus(true);

    match opts.subcommand {
        SubCommand::Server { target, addr } => {
            let server = distfs::server::Server::new(target)?;
            Server::builder()
                .add_service(FilesystemServer::new(server))
                .serve(addr)
                .await?;
        }
        SubCommand::Client {
            uri,
            mountpoint,
            replica,
        } => {
            Session::new(mount_options)
                .mount_with_unprivileged(
                    distfs::client::Distfs::new(
                        uri,
                        replica.unwrap_or(NonZeroUsize::new(16).unwrap()),
                    )
                    .await?,
                    mountpoint,
                )
                .await?
                .await?;
        }
    }
    Ok(())
}
