use byteorder::{ReadBytesExt, WriteBytesExt};
use clap::Parser;
use fuse3::path::prelude::*;
use fuse3::path::PathFilesystem;
use fuse3::{Errno, MountOptions};
use futures::StreamExt;
use futures_util::stream::Iter;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::Permissions;
use std::io;
use std::iter::Skip;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::prelude::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::vec::IntoIter;
use tokio::fs;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReadDirStream;
use tracing::info;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(clap::Parser, Debug)]
struct Opts {
    #[clap(short, long)]
    mountpoint: PathBuf,
    #[clap(short, long)]
    target: PathBuf,
}

struct Distfs {
    mountpoint: PathBuf,
    fh_counter: Arc<Mutex<u64>>,
    file_handlers: Arc<Mutex<HashMap<u64, File>>>,
}

impl Distfs {
    fn new(mountpoint: PathBuf) -> Self {
        Self {
            mountpoint,
            fh_counter: Arc::new(Mutex::new(0)),
            file_handlers: Arc::new(Mutex::new(Default::default())),
        }
    }

    async fn alloc_fh(&self) -> u64 {
        let mut fh = self.fh_counter.lock().await;
        *fh += 1;
        *fh
    }

    fn path(&self, path: &OsStr) -> PathBuf {
        let path = path.to_string_lossy();
        let path = path.strip_prefix('/').unwrap_or(&path);
        self.mountpoint.join(path)
    }
}

fn fuse3_filetype_from_std(filetype: std::fs::FileType) -> fuse3::FileType {
    if filetype.is_fifo() {
        FileType::NamedPipe
    } else if filetype.is_block_device() {
        FileType::BlockDevice
    } else if filetype.is_char_device() {
        FileType::CharDevice
    } else if filetype.is_dir() {
        FileType::Directory
    } else if filetype.is_socket() {
        FileType::Socket
    } else if filetype.is_symlink() {
        FileType::Symlink
    } else {
        FileType::RegularFile
    }
}

fn permission_from_std(permissions: Permissions) -> u16 {
    (permissions.mode() & 0o777) as u16
}

fn map_io_err(err: io::Error) -> fuse3::Errno {
    if let Some(err) = err.raw_os_error() {
        Errno::from(err)
    } else {
        Errno::from(libc::EACCES)
    }
}

fn systemtime_from_raw_unixtime(sec: i64, nsecs: i64) -> SystemTime {
    let epoch = SystemTime::UNIX_EPOCH;
    epoch + Duration::from_secs(sec as u64) + Duration::from_nanos(nsecs as u64)
}

fn attr_from_metadata(metadata: &std::fs::Metadata) -> FileAttr {
    FileAttr {
        size: metadata.len(),
        blocks: metadata.blocks(),
        blksize: metadata.blksize() as u32,
        atime: systemtime_from_raw_unixtime(metadata.atime(), metadata.atime_nsec()),
        ctime: systemtime_from_raw_unixtime(metadata.ctime(), metadata.ctime_nsec()),
        mtime: systemtime_from_raw_unixtime(metadata.mtime(), metadata.mtime_nsec()),
        kind: fuse3_filetype_from_std(metadata.file_type()),
        perm: permission_from_std(metadata.permissions()),
        uid: metadata.uid(),
        gid: metadata.gid(),
        nlink: metadata.nlink() as u32,
        rdev: metadata.rdev() as u32,
    }
}

async fn read_attr<P: AsRef<Path>>(path: P) -> io::Result<FileAttr> {
    let metadata = fs::metadata(path).await?;
    Ok(attr_from_metadata(&metadata))
}

#[async_trait::async_trait]
impl PathFilesystem for Distfs {
    type DirEntryStream = Iter<Skip<IntoIter<fuse3::Result<DirectoryEntry>>>>;
    type DirEntryPlusStream = Iter<Skip<IntoIter<fuse3::Result<DirectoryEntryPlus>>>>;

    async fn init(&self, _req: Request) -> fuse3::Result<()> {
        Ok(())
    }

    async fn setlk(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _type: u32,
        _pid: u32,
        _block: bool,
    ) -> fuse3::Result<()> {
        Err(Errno::from(libc::EACCES))
    }

    async fn getlk(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _type: u32,
        _pid: u32,
    ) -> fuse3::Result<ReplyLock> {
        Err(Errno::from(libc::EACCES))
    }

    async fn destroy(&self, _req: Request) {}

    async fn lookup(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
    ) -> fuse3::Result<ReplyEntry> {
        let path = self.path(parent).join(name);
        Ok(ReplyEntry {
            ttl: Duration::from_secs(1),
            attr: read_attr(path).await?,
        })
    }

    async fn getattr(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        _fh: Option<u64>,
        _flags: u32,
    ) -> fuse3::Result<ReplyAttr> {
        let path = path.ok_or_else(Errno::new_not_exist)?;
        let path = self.path(path);
        let attr = read_attr(&path).await?;
        Ok(ReplyAttr {
            ttl: Duration::from_millis(1000),
            attr,
        })
    }

    async fn open(&self, _req: Request, path: &OsStr, u32_flags: u32) -> fuse3::Result<ReplyOpen> {
        let fh = self.alloc_fh().await;
        let path = self.path(path);
        let mut buf = [0u8; 4];
        (&mut buf[..])
            .write_u32::<byteorder::LE>(u32_flags)
            .unwrap();
        let flags: i32 =
            ReadBytesExt::read_i32::<byteorder::LE>(&mut io::Cursor::new(buf)).unwrap();
        let mut options = tokio::fs::OpenOptions::new();
        options
            .read(
                (flags & libc::O_ACCMODE == libc::O_RDONLY)
                    || (flags & libc::O_ACCMODE == libc::O_RDWR),
            )
            .write(
                (flags & libc::O_ACCMODE == libc::O_WRONLY)
                    || (flags & libc::O_ACCMODE == libc::O_RDWR),
            )
            .append(flags & libc::O_APPEND != 0)
            .truncate(flags & libc::O_TRUNC != 0)
            .create(flags & libc::O_CREAT != 0);
        info!("{:?}", options);
        let file = options.open(path).await.map_err(map_io_err)?;
        self.file_handlers.lock().await.insert(fh, file);
        info!("open success");
        Ok(ReplyOpen {
            fh: fh,
            flags: u32_flags,
        })
    }

    async fn read(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        size: u32,
    ) -> fuse3::Result<ReplyData> {
        let mut file_handlers = self.file_handlers.lock().await;
        let Some(file) = file_handlers.get_mut(&fh) else {
            return Err(Errno::from(libc::ENOENT));
        };
        file.seek(SeekFrom::Start(offset)).await.map_err(|e| {
            if let Some(err) = e.raw_os_error() {
                Errno::from(err)
            } else {
                Errno::from(libc::EACCES)
            }
        })?;
        let mut data = Vec::new();
        data.resize(size as usize, 0);
        file.read_exact(&mut data).await.map_err(|e| {
            if let Some(err) = e.raw_os_error() {
                Errno::from(err)
            } else {
                Errno::from(libc::EACCES)
            }
        })?;
        Ok(ReplyData { data: data.into() })
    }

    async fn access(&self, _req: Request, _path: &OsStr, _mask: u32) -> fuse3::Result<()> {
        Ok(())
    }

    async fn readdirplus(
        &self,
        _req: Request,
        parent: &OsStr,
        _fh: u64,
        offset: u64,
        _lock_owner: u64,
    ) -> fuse3::Result<ReplyDirectoryPlus<Self::DirEntryPlusStream>> {
        let path = self.path(parent);
        info!("readdir: {:?}", path);
        let mut pre_children = vec![Ok::<_, fuse3::Errno>(DirectoryEntryPlus {
            kind: FileType::Directory,
            name: OsString::from("."),
            offset: 1,
            attr: read_attr(&path).await?,
            entry_ttl: Duration::from_millis(1000),
            attr_ttl: Duration::from_millis(1000),
        })];
        if let Some(parent_parent) = path.parent() {
            pre_children.push(Ok(DirectoryEntryPlus {
                kind: FileType::Directory,
                name: OsString::from(".."),
                offset: 2,
                attr: read_attr(parent_parent).await?,
                entry_ttl: Duration::from_millis(1000),
                attr_ttl: Duration::from_millis(1000),
            }))
        };
        let pre_children_len = pre_children.len() as i64;
        let entries = ReadDirStream::new(fs::read_dir(path).await.map_err(map_io_err)?);
        let entries = entries
            .enumerate()
            .then(|(offset, entry)| async move {
                let entry = entry.map_err(map_io_err)?;
                let meta = entry.metadata().await.map_err(map_io_err)?;
                let kind = entry.file_type().await.map_err(map_io_err)?;
                let kind = fuse3_filetype_from_std(kind);
                Ok::<_, fuse3::Errno>(DirectoryEntryPlus {
                    kind,
                    name: entry.file_name(),
                    offset: offset as i64 + 1 + pre_children_len,
                    attr: attr_from_metadata(&meta),
                    entry_ttl: Duration::from_millis(1000),
                    attr_ttl: Duration::from_millis(1000),
                })
            })
            .collect::<Vec<_>>()
            .await;
        let entries = pre_children.into_iter().chain(entries).collect::<Vec<_>>();
        let entries = futures_util::stream::iter(entries.into_iter().skip(offset as _));
        Ok(ReplyDirectoryPlus { entries })
    }

    async fn write(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        data: &[u8],
        _flags: u32,
    ) -> fuse3::Result<ReplyWrite> {
        let mut handlers = self.file_handlers.lock().await;
        let fh = handlers.get_mut(&fh).ok_or_else(Errno::new_not_exist)?;
        fh.seek(SeekFrom::Start(offset)).await.map_err(map_io_err)?;
        fh.write_all(data).await.map_err(map_io_err)?;
        Ok(ReplyWrite {
            written: data.len() as u32,
        })
    }

    async fn statfs(&self, _req: Request, path: &OsStr) -> fuse3::Result<ReplyStatFs> {
        let path = self.path(path);
        let stats = nix::sys::statfs::statfs(&path).map_err(|errno| Errno::from(errno as i32))?;
        Ok(ReplyStatFs {
            blocks: stats.blocks(),
            bfree: stats.blocks_free(),
            bavail: stats.blocks_available(),
            files: stats.files(),
            ffree: stats.files_free(),
            bsize: stats.block_size() as u32,
            namelen: stats.maximum_name_length() as u32,
            frsize: stats.optimal_transfer_size() as u32,
        })
    }
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
        .mount_with_unprivileged(Distfs::new(target), opts.mountpoint)
        .await?
        .await?;
    Ok(())
}
