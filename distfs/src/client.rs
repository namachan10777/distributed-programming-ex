// written by fuse

use std::{
    ffi::{CString, OsStr, OsString},
    num::NonZeroUsize,
    time::Duration,
};

use fuse3::{path::prelude::*, Errno};
use futures::StreamExt;
use tonic::transport::{Channel, Uri};
use tracing::{debug, warn};

use crate::{
    mux,
    proto::fs::{
        attr_response, copy_file_range_response, create_response,
        filesystem_client::{self},
        handle_response, lseek_response, read_response, readdir_response, readlink_response,
        unit_response, write_response, AccessRequest, CopyFileRangeRequest, CreateRequest,
        FallocateRequest, FlushRequest, FsyncRequest, GetAttrRequest, LookupRequest, LseekRequest,
        MkdirRequest, OpenRequest, OpendirRequest, ReadRequest, ReaddirRequest, ReadlinkRequest,
        ReleaseRequest, ReleasedirRequest, RenameRequest, RmdirRequest, SetAttrRequest,
        SymlinkRequest, UnlinkRequest, WriteRequest,
    },
    type_conv,
};

pub struct Distfs {
    client: mux::Pool<filesystem_client::FilesystemClient<Channel>>,
}

impl Distfs {
    pub async fn new(uri: Uri, replica: NonZeroUsize) -> Result<Self, tonic::transport::Error> {
        let client = mux::Pool::new(
            futures::stream::iter(0..replica.get())
                .then(|_| filesystem_client::FilesystemClient::connect(uri.clone()))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<_, _>>()?,
        )
        .await;
        Ok(Self { client })
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
            .get()
            .await
            .lookup(LookupRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        let (ttl_ms, attr) = match lookup.into_inner().result {
            Some(attr_response::Result::Errno(e)) => Err(e.into()),
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                Ok((ttl_ms, attr))
            }
            None => Err(Errno::from(libc::EIO)),
        }?;
        let attr = attr.ok_or(Errno::from(libc::EIO))?;
        Ok(ReplyEntry {
            ttl: std::time::Duration::from_millis(ttl_ms),
            attr: type_conv::attr::grpc_to_fuse3(attr)?,
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
            .get()
            .await
            .get_attr(GetAttrRequest {
                path: path.map(|path| path.to_string_lossy().to_string()),
                fh,
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        let (ttl_ms, attr) = match get_attr.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EIO))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EIO)),
        }?;
        Ok(ReplyAttr {
            ttl: Duration::from_millis(ttl_ms),
            attr: type_conv::attr::grpc_to_fuse3(attr)?,
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
            .get()
            .await
            .set_attr(SetAttrRequest {
                path: path.map(|s| s.to_string_lossy().to_string()),
                fh,
                attr: Some(type_conv::set_attr::fuse3_to_grpc(set_attr)?),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        let (ttl_ms, attr) = match set_attr.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EIO))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EIO)),
        }?;
        Ok(ReplyAttr {
            ttl: Duration::from_millis(ttl_ms),
            attr: type_conv::attr::grpc_to_fuse3(attr)?,
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
            .get()
            .await
            .mkdir(MkdirRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
                mode,
                umask,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        let (ttl_ms, attr) = match set_attr.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EIO))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EIO)),
        }?;
        Ok(ReplyEntry {
            ttl: Duration::from_millis(ttl_ms),
            attr: type_conv::attr::grpc_to_fuse3(attr)?,
        })
    }

    async fn unlink(&self, _req: Request, parent: &OsStr, name: &OsStr) -> fuse3::Result<()> {
        let unlink = self
            .client
            .get()
            .await
            .unlink(UnlinkRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = unlink
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?
        {
            return Err(Errno::from(e));
        }
        Ok(())
    }
    async fn readlink(&self, _req: Request, path: &OsStr) -> fuse3::Result<ReplyData> {
        warn!("readlink");
        let res = self
            .client
            .get()
            .await
            .readlink(ReadlinkRequest {
                path: path.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_e| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        match res {
            readlink_response::Result::Path(path) => {
                let c_str = CString::new(path).map_err(|_e| Errno::from(libc::EIO))?;
                Ok(ReplyData {
                    data: c_str.as_bytes().to_vec().into(),
                })
            }
            readlink_response::Result::Errno(e) => Err(Errno::from(e)),
        }
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
            .get()
            .await
            .symlink(SymlinkRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
                link_path: link_path.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        let (ttl_ms, attr) = match symlink.into_inner().result {
            Some(attr_response::Result::Ok(attr_response::Ok { ttl_ms, attr })) => {
                if let Some(attr) = attr {
                    Ok((ttl_ms, attr))
                } else {
                    Err(Errno::from(libc::EIO))
                }
            }
            Some(attr_response::Result::Errno(e)) => Err(Errno::from(e)),
            None => Err(Errno::from(libc::EIO)),
        }?;
        Ok(ReplyEntry {
            ttl: Duration::from_millis(ttl_ms),
            attr: type_conv::attr::grpc_to_fuse3(attr)?,
        })
    }

    async fn rmdir(&self, _req: Request, parent: &OsStr, name: &OsStr) -> fuse3::Result<()> {
        let rmdir = self
            .client
            .get()
            .await
            .rmdir(RmdirRequest {
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = rmdir
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?
        {
            return Err(Errno::from(e));
        }
        Ok(())
    }
    async fn rename(
        &self,
        req: Request,
        origin_parent: &OsStr,
        origin_name: &OsStr,
        parent: &OsStr,
        name: &OsStr,
    ) -> fuse3::Result<()> {
        self.rename2(req, origin_parent, origin_name, parent, name, 0)
            .await
    }
    async fn open(&self, _req: Request, path: &OsStr, flags: u32) -> fuse3::Result<ReplyOpen> {
        debug!(
            path = path.to_string_lossy().to_string(),
            flags = flags,
            "open_flag"
        );
        let opendir = self
            .client
            .get()
            .await
            .open(OpenRequest {
                path: path.to_string_lossy().to_string(),
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?;
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
            .get()
            .await
            .read(ReadRequest { fh, offset, size })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?;
        match read {
            read_response::Result::Ok(data) => Ok(ReplyData { data: data.into() }),
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
            .get()
            .await
            .write(WriteRequest {
                data: data.to_vec(),
                fh,
                offset,
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        match written {
            write_response::Result::Ok(written) => Ok(ReplyWrite { written }),
            write_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
    async fn release(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        flags: u32,
        lock_owner: u64,
        flush: bool,
    ) -> fuse3::Result<()> {
        let res = self
            .client
            .get()
            .await
            .release(ReleaseRequest {
                flush,
                fh,
                flags,
                lock_owner,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        match res {
            unit_response::Result::Errno(e) => Err(Errno::from(e)),
            unit_response::Result::Ok(()) => Ok(()),
        }
    }
    async fn fsync(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        datasync: bool,
    ) -> fuse3::Result<()> {
        let res = self
            .client
            .get()
            .await
            .fsync(FsyncRequest { fh, datasync })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = res {
            return Err(Errno::from(e));
        }
        Ok(())
    }
    async fn flush(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        lock_owner: u64,
    ) -> fuse3::Result<()> {
        let res = self
            .client
            .get()
            .await
            .flush(FlushRequest { fh, lock_owner })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = res {
            Err(Errno::from(e))
        } else {
            Ok(())
        }
    }
    async fn access(&self, _req: Request, path: &OsStr, mask: u32) -> fuse3::Result<()> {
        let res = self
            .client
            .get()
            .await
            .access(AccessRequest {
                path: path.to_string_lossy().to_string(),
                mask,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = res
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?
        {
            return Err(Errno::from(e));
        } else {
            Ok(())
        }
    }
    async fn create(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
        mode: u32,
        flags: u32,
    ) -> fuse3::Result<ReplyCreated> {
        let res = self
            .client
            .get()
            .await
            .create(CreateRequest {
                flags,
                name: name.to_string_lossy().to_string(),
                mode,
                parent: parent.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        match res {
            create_response::Result::Ok(create_response::Ok {
                ttl_ms,
                attr,
                generation,
                fh,
                flags,
            }) => Ok(ReplyCreated {
                ttl: Duration::from_millis(ttl_ms),
                attr: type_conv::attr::grpc_to_fuse3(attr.ok_or(libc::EIO)?)?,
                generation,
                fh,
                flags,
            }),
            create_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
    async fn batch_forget(&self, _req: Request, _paths: &[&OsStr]) {}

    #[cfg(target_os = "linux")]
    async fn fallocate(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        length: u64,
        mode: u32,
    ) -> fuse3::Result<()> {
        let res = self
            .client
            .get()
            .await
            .fallocate(FallocateRequest {
                fh,
                offset,
                length,
                mode,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = res {
            return Err(Errno::from(e));
        } else {
            Ok(())
        }
    }

    async fn opendir(&self, _req: Request, path: &OsStr, flags: u32) -> fuse3::Result<ReplyOpen> {
        let opendir = self
            .client
            .get()
            .await
            .opendir(OpendirRequest {
                path: path.to_string_lossy().to_string(),
                flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?;
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
            .get()
            .await
            .releasedir(ReleasedirRequest {
                path: path.to_string_lossy().to_string(),
                fh,
                flag: flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = releasedir
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?
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
            .get()
            .await
            .readdir(ReaddirRequest {
                fh,
                offset,
                lock_owner,
                parent: parent.to_string_lossy().to_string(),
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        let entries = match readdir {
            readdir_response::Result::Errno(e) => Err(Errno::from(e)),
            readdir_response::Result::Ok(entries) => Ok(entries.inner),
        }?;
        let entries = entries
            .into_iter()
            .map(|entry| {
                let entry = entry.inner.ok_or(libc::EIO)?;
                let entry = match entry {
                    readdir_response::entry::Inner::Entry(entry) => Ok(entry),
                    readdir_response::entry::Inner::Errno(e) => Err(e),
                }?;
                let kind = entry.kind();
                Ok(DirectoryEntryPlus {
                    attr: type_conv::attr::grpc_to_fuse3(entry.attr.ok_or(libc::EIO)?)?,
                    kind: type_conv::filetype::grpc_to_fuse3(kind),
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
        origin_parent: &OsStr,
        origin_name: &OsStr,
        parent: &OsStr,
        name: &OsStr,
        flags: u32,
    ) -> fuse3::Result<()> {
        let rename = self
            .client
            .get()
            .await
            .rename(RenameRequest {
                origin_parent: origin_parent.to_string_lossy().to_string(),
                origin_name: origin_name.to_string_lossy().to_string(),
                parent: parent.to_string_lossy().to_string(),
                name: name.to_string_lossy().to_string(),
                flag: flags,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?;
        if let unit_response::Result::Errno(e) = rename
            .into_inner()
            .result
            .ok_or_else(|| Errno::from(libc::EIO))?
        {
            Err(Errno::from(e))
        } else {
            Ok(())
        }
    }
    async fn lseek(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        whence: u32,
    ) -> fuse3::Result<ReplyLSeek> {
        let res = self
            .client
            .get()
            .await
            .lseek(LseekRequest { fh, offset, whence })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        match res {
            lseek_response::Result::Offset(offset) => Ok(ReplyLSeek { offset }),
            lseek_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
    async fn copy_file_range(
        &self,
        _req: Request,
        _from_path: Option<&OsStr>,
        fh_in: u64,
        offset_in: u64,
        _to_path: Option<&OsStr>,
        fh_out: u64,
        offset_out: u64,
        length: u64,
        flags: u64,
    ) -> fuse3::Result<ReplyCopyFileRange> {
        let res = self
            .client
            .get()
            .await
            .copy_file_range(CopyFileRangeRequest {
                fh_in,
                fh_out,
                flags,
                offset_in,
                offset_out,
                length,
            })
            .await
            .map_err(|_| Errno::from(libc::EIO))?
            .into_inner()
            .result
            .ok_or(Errno::from(libc::EIO))?;
        match res {
            copy_file_range_response::Result::Copied(copied) => Ok(ReplyCopyFileRange { copied }),
            copy_file_range_response::Result::Errno(e) => Err(Errno::from(e)),
        }
    }
}
