use clap::Parser;
use fuse3::path::prelude::*;
use fuse3::MountOptions;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::prelude::*;

#[derive(clap::Parser, Debug)]
struct Opts {
    #[clap(short, long)]
    mountpoint: PathBuf,
    #[clap(short, long)]
    target: PathBuf,
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

    let target = std::fs::canonicalize(&opts.target)?;
    info!("target: {:?}", target);

    Session::new(mount_options)
        .mount_with_unprivileged(distfs::client::Distfs::new(target), opts.mountpoint)
        .await?
        .await?;
    Ok(())
}
