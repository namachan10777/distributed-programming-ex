use byteorder::{ReadBytesExt, WriteBytesExt};
use clap::Parser;
use fuse3::path::{prelude::*, PathFilesystem};
use fuse3::{Errno, MountOptions};
use futures::StreamExt;
use futures_util::stream::Iter;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::iter::Skip;
use std::os::unix::{
    fs::{FileTypeExt, MetadataExt},
    prelude::PermissionsExt,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use std::vec::IntoIter;
use tokio::fs;
use tokio::io::{AsyncWriteExt, AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReadDirStream;
use tracing::{error, info, warn};
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
