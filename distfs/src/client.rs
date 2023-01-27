// written by fuse

use std::{
    ffi::{OsStr, OsString},
    sync::Arc,
    time::Duration,
};

use fuse3::{path::prelude::*, Errno};
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tracing::debug;

use crate::proto::{
    self,
    fs::{
        attr_grpc_to_fuse3, attr_response, filesystem_client::FilesystemClient,
        ftype_grpc_to_fuse3, handle_response, read_response, readdir_response,
        settableattr_fuse3_to_grpc, unit_response, write_response, GetAttrRequest, LookupRequest,
        MkdirRequest, OpenRequest, OpendirRequest, ReadRequest, ReaddirRequest, ReleasedirRequest,
        RenameRequest, RmdirRequest, SetAttrRequest, SymlinkRequest, UnlinkRequest, WriteRequest,
    },
};

pub struct Distfs {
    client: Arc<Mutex<proto::fs::filesystem_client::FilesystemClient<Channel>>>,
}

impl Distfs {
    pub async fn new(
        grpc_endpoint: tonic::transport::Uri,
    ) -> Result<Self, tonic::transport::Error> {
        let client = FilesystemClient::connect(grpc_endpoint.clone()).await?;
        let client = Arc::new(Mutex::new(client));
        Ok(Distfs { client })
    }
}

#[async_trait::async_trait]
impl PathFilesystem for Distfs {
    type DirEntryStream = futures_util::stream::Empty<fuse3::Result<DirectoryEntry>>;
    type DirEntryPlusStream =
        futures_util::stream::Iter<std::vec::IntoIter<fuse3::Result<DirectoryEntryPlus>>>;

    async fn init(&self, _req: Request) -> fuse3::Result<()> {
        Ok(())
    }
    async fn destroy(&self, _req: Request) {}
    async fn lookup(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
    ) -> fuse3::Result<ReplyEntry> {
        let lookup = self
            .client
            .lock()
            .await
            .lookup(LookupRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        let (ttl_ms, attr) = match lookup.into_inner().result {
            Some(attr_response::Result::Errno(e)) => Err(e.into()),
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                Ok((ttl_ms, attr))
            }
            None => Err(Errno::from(libc::EACCES)),
        }?;
        let attr = attr.ok_or(Errno::from(libc::EACCES))?;
        Ok(ReplyEntry {
            ttl: std::time::Duration::from_millis(ttl_ms),
            attr: crate::proto::fs::attr_grpc_to_fuse3(attr)?,
        })
    }

    async fn forget(&self, _req: Request, _parent: &OsStr, _nlookup: u64) {}
    async fn getattr(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: Option<u64>,
        flags: u32,
    ) -> fuse3::Result<ReplyAttr> {
        let get_attr = self
            .client
            .lock()
            .await
            .get_attr(GetAttrRequest {
                path: path.map(|path| path.to_string_lossy().to_string()),
                fh,
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        let (ttl_ms, attr) = match get_attr.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EACCES))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EACCES)),
        }?;
        Ok(ReplyAttr {
            ttl: Duration::from_millis(ttl_ms),
            attr: crate::proto::fs::attr_grpc_to_fuse3(attr)?,
        })
    }

    async fn setattr(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: Option<u64>,
        set_attr: SetAttr,
    ) -> fuse3::Result<ReplyAttr> {
        let set_attr = self
            .client
            .lock()
            .await
            .set_attr(SetAttrRequest {
                path: path.map(|s| s.to_string_lossy().to_string()),
                fh,
                attr: Some(settableattr_fuse3_to_grpc(set_attr)?),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        let (ttl_ms, attr) = match set_attr.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EACCES))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EACCES)),
        }?;
        Ok(ReplyAttr {
            ttl: Duration::from_millis(ttl_ms),
            attr: crate::proto::fs::attr_grpc_to_fuse3(attr)?,
        })
    }

    async fn mkdir(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
        mode: u32,
        umask: u32,
    ) -> fuse3::Result<ReplyEntry> {
        let set_attr = self
            .client
            .lock()
            .await
            .mkdir(MkdirRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
                mode,
                umask,
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        let (ttl_ms, attr) = match set_attr.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EACCES))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EACCES)),
        }?;
        Ok(ReplyEntry {
            ttl: Duration::from_millis(ttl_ms),
            attr: crate::proto::fs::attr_grpc_to_fuse3(attr)?,
        })
    }

    async fn unlink(&self, _req: Request, parent: &OsStr, name: &OsStr) -> fuse3::Result<()> {
        let unlink = self
            .client
            .lock()
            .await
            .unlink(UnlinkRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        if let unit_response::Result::Errno(e) = unlink
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?
        {
            return Err(Errno::from(e));
        }
        Ok(())
    }
    async fn readlink(&self, _req: Request, _path: &OsStr) -> fuse3::Result<ReplyData> {
        // TODO
        Err(Errno::from(libc::ENOSYS))
    }
    async fn symlink(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
        link_path: &OsStr,
    ) -> fuse3::Result<ReplyEntry> {
        let symlink = self
            .client
            .lock()
            .await
            .symlink(SymlinkRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
                link_path: link_path.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        let (ttl_ms, attr) = match symlink.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EACCES))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EACCES)),
        }?;
        Ok(ReplyEntry {
            ttl: Duration::from_millis(ttl_ms),
            attr: crate::proto::fs::attr_grpc_to_fuse3(attr)?,
        })
    }

    async fn rmdir(&self, _req: Request, parent: &OsStr, name: &OsStr) -> fuse3::Result<()> {
        let rmdir = self
            .client
            .lock()
            .await
            .rmdir(RmdirRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        if let unit_response::Result::Errno(e) = rmdir
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?
        {
            return Err(Errno::from(e));
        }
        Ok(())
    }
    async fn rename(
        &self,
        _req: Request,
        origin_parent: &OsStr,
        origin_name: &OsStr,
        parent: &OsStr,
        name: &OsStr,
    ) -> fuse3::Result<()> {
        let rename = self
            .client
            .lock()
            .await
            .rename(RenameRequest {
                origin_parent: origin_parent.to_string_lossy().to_string(),
                origin_name: origin_name.to_string_lossy().to_string(),
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        if let unit_response::Result::Errno(e) = rename
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?
        {
            return Err(Errno::from(e));
        }
        Ok(())
    }
    async fn open(&self, _req: Request, path: &OsStr, flags: u32) -> fuse3::Result<ReplyOpen> {
        debug!(
            path = path.to_string_lossy().to_string(),
            flags = flags,
            "open_flag"
        );
        let opendir = self
            .client
            .lock()
            .await
            .open(OpenRequest {
                path: path.to_string_lossy().to_string(),
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?;
        match opendir {
            handle_response::Result::Ok(handle_response::Ok { fh, flag }) => {
                Ok(ReplyOpen { fh, flags: flag })
            }
            handle_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
    async fn read(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        size: u32,
    ) -> fuse3::Result<ReplyData> {
        let read = self
            .client
            .lock()
            .await
            .read(ReadRequest { fh, offset, size })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?;
        match read {
            read_response::Result::Ok(read_response::Ok { data }) => {
                Ok(ReplyData { data: data.into() })
            }
            read_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
    async fn write(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        data: &[u8],
        flags: u32,
    ) -> fuse3::Result<ReplyWrite> {
        let written = self
            .client
            .lock()
            .await
            .write(WriteRequest {
                data: data.to_vec(),
                fh,
                offset,
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EACCES))?;
        match written {
            write_response::Result::Ok(written) => Ok(ReplyWrite { written }),
            write_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
    async fn release(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
    ) -> fuse3::Result<()> {
        Err(Errno::from(libc::ENOSYS))
    }
    async fn fsync(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _datasync: bool,
    ) -> fuse3::Result<()> {
        Err(Errno::from(libc::ENOSYS))
    }
    async fn flush(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _lock_owner: u64,
    ) -> fuse3::Result<()> {
        Err(Errno::from(libc::ENOSYS))
    }
    async fn access(&self, _req: Request, _path: &OsStr, _mask: u32) -> fuse3::Result<()> {
        Err(libc::ENOSYS.into())
    }
    async fn create(
        &self,
        _req: Request,
        _parent: &OsStr,
        _name: &OsStr,
        _mode: u32,
        _flags: u32,
    ) -> fuse3::Result<ReplyCreated> {
        Err(libc::ENOSYS.into())
    }
    async fn batch_forget(&self, _req: Request, _paths: &[&OsStr]) {}

    #[cfg(target_os = "linux")]
    async fn fallocate(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _offset: u64,
        _length: u64,
        _mode: u32,
    ) -> fuse3::Result<()> {
        Err(libc::ENOSYS.into())
    }

    async fn opendir(&self, _req: Request, path: &OsStr, flags: u32) -> fuse3::Result<ReplyOpen> {
        let opendir = self
            .client
            .lock()
            .await
            .opendir(OpendirRequest {
                path: path.to_string_lossy().to_string(),
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?;
        match opendir {
            handle_response::Result::Ok(handle_response::Ok { fh, flag }) => {
                Ok(ReplyOpen { fh, flags: flag })
            }
            handle_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }

    async fn releasedir(
        &self,
        _req: Request,
        path: &OsStr,
        fh: u64,
        flags: u32,
    ) -> fuse3::Result<()> {
        let releasedir = self
            .client
            .lock()
            .await
            .releasedir(ReleasedirRequest {
                path: path.to_string_lossy().to_string(),
                fh,
                flag: flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?;
        if let unit_response::Result::Errno(e) = releasedir
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EACCES))?
        {
            return Err(Errno::from(e));
        }
        Ok(())
    }

    async fn readdirplus(
        &self,
        req: Request,
        parent: &OsStr,
        fh: u64,
        offset: u64,
        lock_owner: u64,
    ) -> fuse3::Result<ReplyDirectoryPlus<Self::DirEntryPlusStream>> {
        let readdir = self
            .client
            .lock()
            .await
            .readdir(ReaddirRequest {
                fh,
                offset,
                lock_owner,
                parent: parent.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EACCES))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EACCES))?;
        let entries = match readdir {
            readdir_response::Result::Errno(e) => Err(Errno::from(e)),
            readdir_response::Result::Ok(entries) => Ok(entries.inner),
        }?;
        let entries = entries
            .into_iter()
            .map(|entry| {
                let entry = entry.inner.ok_or(libc::EACCES)?;
                let entry = match entry {
                    readdir_response::entry::Inner::Entry(entry) => Ok(entry),
                    readdir_response::entry::Inner::Errno(e) => Err(e),
                }?;
                let kind = entry.kind();
                Ok(DirectoryEntryPlus {
                    attr: attr_grpc_to_fuse3(entry.attr.ok_or(libc::EACCES)?)?,
                    kind: ftype_grpc_to_fuse3(kind),
                    name: OsString::from(entry.name),
                    offset: entry.offset,
                    attr_ttl: Duration::from_millis(entry.attr_ttl_ms),
                    entry_ttl: Duration::from_millis(entry.entry_ttl_ms),
                })
            })
            .collect::<Vec<fuse3::Result<DirectoryEntryPlus>>>();
        for entry in &entries {
            debug!(id = req.unique, offset = format!("{entry:?}"), "readdir");
        }
        Ok(ReplyDirectoryPlus {
            entries: futures::stream::iter(entries.into_iter()),
        })
    }
    async fn rename2(
        &self,
        _req: Request,
        _origin_parent: &OsStr,
        _origin_name: &OsStr,
        _parent: &OsStr,
        _name: &OsStr,
        _flags: u32,
    ) -> fuse3::Result<()> {
        Err(libc::ENOSYS.into())
    }
    async fn lseek(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _offset: u64,
        _whence: u32,
    ) -> fuse3::Result<ReplyLSeek> {
        Err(libc::ENOSYS.into())
    }
    async fn copy_file_range(
        &self,
        _req: Request,
        _from_path: Option<&OsStr>,
        _fh_in: u64,
        _offset_in: u64,
        _to_path: Option<&OsStr>,
        _fh_out: u64,
        _offset_out: u64,
        _length: u64,
        _flags: u64,
    ) -> fuse3::Result<ReplyCopyFileRange> {
        Err(libc::ENOSYS.into())
    }
}
