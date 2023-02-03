use std::{
    collections::HashMap,
    fs::Permissions,
    io,
    os::unix::prelude::PermissionsExt,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use file_owner::{FileOwnerError, PathExt};
use filetime::{set_file_atime, set_file_mtime, FileTime};
use futures::StreamExt;
use nix::{
    sys::stat::Mode,
    unistd::{mkdir, unlink},
};
use tokio::{
    fs::{self, remove_dir, rename, set_permissions, symlink, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::RwLock,
};
use tonic::Response;
use tracing::debug;

use crate::{
    proto::fs::{
        attr_response, handle_response, read_response,
        readdir_response::{self, Entry},
        unit_response, write_response, Attr, AttrResponse, GetAttrRequest, HandleResponse,
        OpenRequest, OpendirRequest, ReadRequest, ReadResponse, ReaddirRequest, ReaddirResponse,
        ReleasedirRequest, SetAttrRequest, SettableAttr, UnitResponse, WriteRequest, WriteResponse,
    },
    type_conv,
};

use super::proto;
pub struct Server {
    root_path: PathBuf,
    fh: RwLock<HashMap<u64, (tokio::fs::File, std::path::PathBuf)>>,
    dh: RwLock<HashMap<u64, (Vec<readdir_response::Entry>, std::path::PathBuf)>>,
    fh_src: AtomicU64,
}

impl Server {
    pub fn new(root_path: PathBuf) -> io::Result<Server> {
        Ok(Server {
            root_path: root_path.canonicalize()?,
            fh: RwLock::new(HashMap::new()),
            dh: RwLock::new(HashMap::new()),
            fh_src: AtomicU64::new(0),
        })
    }

    fn real_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        path.as_ref()
            .components()
            .fold(
                self.root_path.clone(),
                |parent, component| match component {
                    Component::RootDir => self.root_path.clone(),
                    Component::ParentDir => {
                        if parent == self.root_path {
                            parent
                        } else {
                            parent
                                .parent()
                                .map(|p| p.to_owned())
                                .unwrap_or_else(|| self.root_path.clone())
                        }
                    }
                    Component::CurDir => parent,
                    Component::Normal(component) => parent.join(component),
                    Component::Prefix(_) => self.root_path.clone(),
                },
            )
    }

    fn alloc_fh(&self) -> u64 {
        self.fh_src.fetch_add(1, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn path<P: AsRef<Path>>(path: P) -> PathBuf {
        path.as_ref().to_owned()
    }

    #[test]
    fn test_real_path() {
        let server = Server::new(path("src")).unwrap();
        assert_eq!(server.real_path("/"), server.root_path);
        assert_eq!(server.real_path("."), server.root_path);
        assert_eq!(server.real_path("../"), server.root_path);
        assert_eq!(server.real_path("../foo"), server.root_path.join("foo"));
        assert_eq!(
            server.real_path("hoge/bar"),
            server.root_path.join("hoge").join("bar")
        );
    }
}

fn file_owner_error_to_io_error(e: FileOwnerError) -> std::io::Error {
    match e {
        FileOwnerError::IoError(e) => e,
        FileOwnerError::NixError(e) => io::Error::from_raw_os_error(e as i32),
        FileOwnerError::GroupNotFound(group) => io::Error::new(io::ErrorKind::NotFound, group),
        FileOwnerError::UserNotFound(user) => io::Error::new(io::ErrorKind::NotFound, user),
    }
}

async fn set_attr_by_path<P: AsRef<Path>>(path: P, attr: SettableAttr) -> std::io::Result<()> {
    if let Some(uid) = attr.uid {
        path.set_owner(uid).map_err(file_owner_error_to_io_error)?;
    }
    if let Some(gid) = attr.gid {
        path.set_group(gid).map_err(file_owner_error_to_io_error)?;
    }
    if let Some(size) = attr.size {
        let file = fs::File::open(path.as_ref()).await?;
        file.set_len(size).await?;
    }
    if let Some(mtime) = attr.mtime {
        set_file_mtime(
            path.as_ref(),
            FileTime::from_unix_time(mtime.seconds, mtime.nanos as u32),
        )?;
    }
    if let Some(_ctime) = attr.ctime {
        // ctime is unimplemented
    }
    if let Some(atime) = attr.atime {
        set_file_atime(
            path.as_ref(),
            FileTime::from_unix_time(atime.seconds, atime.nanos as u32),
        )?;
    }
    if let Some(mode) = attr.mode {
        set_permissions(path, Permissions::from_mode(mode)).await?;
    }
    if attr.lock_owner.is_some() {
        // lock is unimplemented
    }
    Ok(())
}

impl Server {
    async fn get_attr_from_path<P: AsRef<Path>>(&self, path: P) -> Result<Attr, libc::c_int> {
        let meta = fs::metadata(&path)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        Ok(type_conv::attr::metadata_to_grpc(meta))
    }

    async fn lookup_impl(
        &self,
        req: tonic::Request<proto::fs::LookupRequest>,
    ) -> Result<Attr, libc::c_int> {
        let req = req.into_inner();
        let path = self.real_path(req.parent).join(req.name);
        let attr = self.get_attr_from_path(&path).await?;
        Ok(attr)
    }

    async fn get_attr_impl(
        &self,
        req: tonic::Request<proto::fs::GetAttrRequest>,
    ) -> Result<Attr, libc::c_int> {
        let GetAttrRequest { path, fh, flags: _ } = req.into_inner();
        match (path, fh) {
            (_, Some(fh)) => {
                let lock = self.fh.read().await;
                let file = lock.get(&fh).ok_or(libc::EEXIST)?;
                let meta = file
                    .0
                    .metadata()
                    .await
                    .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
                Ok(type_conv::attr::metadata_to_grpc(meta))
            }
            (Some(path), None) => self.get_attr_from_path(path).await,
            (None, None) => Err(libc::EACCES),
        }
    }

    async fn set_attr_impl(
        &self,
        req: tonic::Request<proto::fs::SetAttrRequest>,
    ) -> Result<Attr, libc::c_int> {
        let req = req.into_inner();
        let SetAttrRequest { path, fh, attr } = req;
        let Some(attr) = attr else {
            return Err(libc::EACCES);
        };
        match (path, fh) {
            (_, Some(fh)) => {
                let lock = self.fh.read().await;
                let (file, path) = lock.get(&fh).ok_or(libc::EEXIST)?;
                set_attr_by_path(self.real_path(path), attr)
                    .await
                    .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
                let meta = file
                    .metadata()
                    .await
                    .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
                Ok(type_conv::attr::metadata_to_grpc(meta))
            }
            (Some(path), _) => {
                set_attr_by_path(self.real_path(&path), attr)
                    .await
                    .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
                self.get_attr_from_path(self.real_path(path)).await
            }
            (None, None) => Err(libc::EACCES),
        }
    }

    async fn mkdir_impl(
        &self,
        req: tonic::Request<proto::fs::MkdirRequest>,
    ) -> Result<Attr, libc::c_int> {
        let req = req.into_inner();
        let path = self.real_path(req.parent).join(req.name);
        mkdir(&path, Mode::from_bits_truncate(req.mode)).map_err(|e| e as libc::c_int)?;
        let meta = fs::metadata(&path)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        Ok(type_conv::attr::metadata_to_grpc(meta))
    }

    async fn unlink_impl(
        &self,
        req: tonic::Request<proto::fs::UnlinkRequest>,
    ) -> Result<(), libc::c_int> {
        let req = req.into_inner();
        let path = self.real_path(req.parent).join(req.name);
        unlink(&path).map_err(|e| e as libc::c_int)?;
        Ok(())
    }

    async fn symlink_impl(
        &self,
        req: tonic::Request<proto::fs::SymlinkRequest>,
    ) -> Result<Attr, libc::c_int> {
        let req = req.into_inner();
        let target = self.real_path(req.parent).join(req.name);
        let source = self.real_path(&req.link_path);
        symlink(source, &target)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        let meta = fs::metadata(target)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        Ok(type_conv::attr::metadata_to_grpc(meta))
    }

    async fn rmdir_impl(
        &self,
        req: tonic::Request<proto::fs::RmdirRequest>,
    ) -> Result<(), libc::c_int> {
        let req = req.into_inner();
        let path = self.real_path(req.parent).join(req.name);
        remove_dir(path)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        Ok(())
    }

    async fn rename_impl(
        &self,
        req: tonic::Request<proto::fs::RenameRequest>,
    ) -> Result<(), libc::c_int> {
        let req = req.into_inner();
        let origin_path = self.real_path(req.origin_parent).join(req.origin_name);
        let path = self.real_path(req.parent).join(req.name);
        rename(origin_path, path)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        Ok(())
    }

    async fn opendir_impl(
        &self,
        req: tonic::Request<OpendirRequest>,
    ) -> Result<(u64, u32), libc::c_int> {
        let req = req.into_inner();
        let fh = self.alloc_fh();
        let path = self.real_path(&req.path);
        debug!(
            path = req.path,
            real_path = path.to_string_lossy().to_string(),
            "opendir"
        );
        let readdir = fs::read_dir(&path)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        let readdir = tokio_stream::wrappers::ReadDirStream::new(readdir);
        let inner = readdir
            .enumerate()
            .then(|(offset, entry)| async move {
                let inner = match entry {
                    Ok(entry) => {
                        let kind =
                            type_conv::filetype::std_to_grpc(entry.file_type().await?).into();
                        let attr = type_conv::attr::metadata_to_grpc(entry.metadata().await?);
                        readdir_response::entry::Inner::Entry(proto::fs::DirEntry {
                            kind,
                            name: entry.file_name().to_string_lossy().to_string(),
                            offset: 1 + offset as i64,
                            attr: Some(attr),
                            entry_ttl_ms: 1000,
                            attr_ttl_ms: 1000,
                        })
                    }
                    Err(e) => readdir_response::entry::Inner::Errno(
                        e.raw_os_error().unwrap_or(libc::EACCES),
                    ),
                };
                Ok(readdir_response::Entry { inner: Some(inner) })
            })
            .map(|entry| {
                entry.unwrap_or_else(|e: std::io::Error| readdir_response::Entry {
                    inner: Some(readdir_response::entry::Inner::Errno(
                        e.raw_os_error().unwrap_or(libc::EACCES),
                    )),
                })
            })
            .collect::<Vec<_>>()
            .await;
        self.dh.write().await.insert(fh, (inner, path));
        Ok((fh, req.flags))
    }

    async fn releasdir_impl(
        &self,
        req: tonic::Request<ReleasedirRequest>,
    ) -> Result<(), libc::c_int> {
        let req = req.into_inner();
        self.dh.write().await.remove(&req.fh);
        Ok(())
    }

    async fn readdir_impl(
        &self,
        req: tonic::Request<ReaddirRequest>,
    ) -> Result<Vec<Entry>, libc::c_int> {
        let req = req.into_inner();
        let lock = self.dh.read().await;
        let (entries, path) = lock.get(&req.fh).ok_or(libc::EEXIST)?;
        debug!(
            offset = req.offset,
            path = path.to_string_lossy().to_string(),
            "readdir"
        );
        let entries = entries.iter().skip(req.offset as usize).cloned().collect();
        Ok(entries)
    }

    async fn open_impl(&self, req: tonic::Request<OpenRequest>) -> Result<(u64, u32), libc::c_int> {
        let req = req.into_inner();
        let path = self.real_path(&req.path);
        let create = req.flags as i32 & libc::O_CREAT != 0;
        let append = req.flags as i32 & libc::O_APPEND != 0;
        let w_only = req.flags as i32 & 0b11 == libc::O_WRONLY;
        let r_only = req.flags as i32 & 0b11 == libc::O_RDONLY;
        let rw = req.flags as i32 & 0b11 == libc::O_RDWR;
        let file = OpenOptions::new()
            .create(create)
            .append(append)
            .write(rw || w_only)
            .read(rw || r_only)
            .open(&path)
            .await
            .map_err(|e| {
                debug!(
                    error = e.to_string(),
                    path = path.to_string_lossy().to_string(),
                    rw = rw,
                    w_only = w_only,
                    r_only = r_only,
                    create = create,
                    append = append,
                    "open_failed"
                );
                e.raw_os_error().unwrap_or(libc::EACCES)
            })?;
        let fh = self.alloc_fh();
        self.fh.write().await.insert(fh, (file, path));
        Ok((fh, req.flags))
    }

    async fn read_impl(&self, req: tonic::Request<ReadRequest>) -> Result<Vec<u8>, libc::c_int> {
        let req = req.into_inner();

        let mut lock = self.fh.write().await;
        let (file, path) = lock.get_mut(&req.fh).ok_or(libc::EEXIST)?;
        let mut data = vec![0u8; req.size as usize];

        debug!(
            path = path.to_string_lossy().to_string(),
            size = req.size,
            offset = req.offset,
            "read"
        );

        let mut size = 0;

        file.seek(io::SeekFrom::Start(req.offset))
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;

        loop {
            let read_size = file
                .read(&mut data[size..])
                .await
                .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
            size += read_size;
            if read_size == 0 {
                break;
            }
        }
        data.resize(size, 0u8);
        let meta = file
            .metadata()
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        debug!(
            size = size,
            data = data.len(),
            meta = meta.len(),
            "read_size"
        );
        Ok(data)
    }

    async fn write_impl(&self, req: tonic::Request<WriteRequest>) -> Result<u32, libc::c_int> {
        let req = req.into_inner();
        let mut lock = self.fh.write().await;
        let (file, _) = lock.get_mut(&req.fh).ok_or(libc::EEXIST)?;
        file.seek(io::SeekFrom::Start(req.offset))
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        let written = file
            .write(&req.data)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))?;
        Ok(written as u32)
    }
}

type RpcResult<T> = Result<tonic::Response<T>, tonic::Status>;

#[async_trait::async_trait]
impl proto::fs::filesystem_server::Filesystem for Server {
    async fn lookup(
        &self,
        req: tonic::Request<proto::fs::LookupRequest>,
    ) -> RpcResult<proto::fs::AttrResponse> {
        Ok(Response::new(AttrResponse {
            result: Some(
                self.lookup_impl(req)
                    .await
                    .map(|attr| {
                        attr_response::Result::Ok(attr_response::Ok {
                            ttl_ms: 1000,
                            attr: Some(attr),
                        })
                    })
                    .unwrap_or_else(|e| attr_response::Result::Errno(e)),
            ),
        }))
    }

    async fn get_attr(
        &self,
        req: tonic::Request<proto::fs::GetAttrRequest>,
    ) -> RpcResult<proto::fs::AttrResponse> {
        Ok(Response::new(AttrResponse {
            result: Some(
                self.get_attr_impl(req)
                    .await
                    .map(|attr| {
                        attr_response::Result::Ok(attr_response::Ok {
                            ttl_ms: 1000,
                            attr: Some(attr),
                        })
                    })
                    .unwrap_or_else(|e| attr_response::Result::Errno(e)),
            ),
        }))
    }

    async fn set_attr(
        &self,
        req: tonic::Request<proto::fs::SetAttrRequest>,
    ) -> RpcResult<proto::fs::AttrResponse> {
        Ok(Response::new(AttrResponse {
            result: Some(
                self.set_attr_impl(req)
                    .await
                    .map(|attr| {
                        attr_response::Result::Ok(attr_response::Ok {
                            ttl_ms: 1000,
                            attr: Some(attr),
                        })
                    })
                    .unwrap_or_else(|e| attr_response::Result::Errno(e)),
            ),
        }))
    }

    async fn mkdir(
        &self,
        req: tonic::Request<proto::fs::MkdirRequest>,
    ) -> RpcResult<proto::fs::AttrResponse> {
        Ok(Response::new(AttrResponse {
            result: Some(
                self.mkdir_impl(req)
                    .await
                    .map(|attr| {
                        attr_response::Result::Ok(attr_response::Ok {
                            ttl_ms: 1000,
                            attr: Some(attr),
                        })
                    })
                    .unwrap_or_else(|e| attr_response::Result::Errno(e)),
            ),
        }))
    }

    async fn unlink(
        &self,
        req: tonic::Request<proto::fs::UnlinkRequest>,
    ) -> RpcResult<UnitResponse> {
        Ok(Response::new(UnitResponse {
            result: Some(
                self.unlink_impl(req)
                    .await
                    .map(|()| unit_response::Result::Ok(()))
                    .unwrap_or_else(|e| unit_response::Result::Errno(e)),
            ),
        }))
    }

    async fn symlink(
        &self,
        req: tonic::Request<proto::fs::SymlinkRequest>,
    ) -> RpcResult<proto::fs::AttrResponse> {
        Ok(Response::new(AttrResponse {
            result: Some(
                self.symlink_impl(req)
                    .await
                    .map(|attr| {
                        attr_response::Result::Ok(attr_response::Ok {
                            ttl_ms: 1000,
                            attr: Some(attr),
                        })
                    })
                    .unwrap_or_else(|e| attr_response::Result::Errno(e)),
            ),
        }))
    }

    async fn rmdir(
        &self,
        req: tonic::Request<proto::fs::RmdirRequest>,
    ) -> RpcResult<proto::fs::UnitResponse> {
        Ok(Response::new(UnitResponse {
            result: Some(
                self.rmdir_impl(req)
                    .await
                    .map(|()| unit_response::Result::Ok(()))
                    .unwrap_or_else(|e| unit_response::Result::Errno(e)),
            ),
        }))
    }

    async fn rename(
        &self,
        req: tonic::Request<proto::fs::RenameRequest>,
    ) -> RpcResult<UnitResponse> {
        Ok(Response::new(UnitResponse {
            result: Some(
                self.rename_impl(req)
                    .await
                    .map(|_| unit_response::Result::Ok(()))
                    .unwrap_or_else(|e| unit_response::Result::Errno(e)),
            ),
        }))
    }

    async fn opendir(&self, req: tonic::Request<OpendirRequest>) -> RpcResult<HandleResponse> {
        Ok(Response::new(HandleResponse {
            result: Some(
                self.opendir_impl(req)
                    .await
                    .map(|(fh, flag)| handle_response::Result::Ok(handle_response::Ok { fh, flag }))
                    .unwrap_or_else(|e| handle_response::Result::Errno(e)),
            ),
        }))
    }

    async fn releasedir(
        &self,
        req: tonic::Request<proto::fs::ReleasedirRequest>,
    ) -> RpcResult<UnitResponse> {
        Ok(Response::new(UnitResponse {
            result: Some(
                self.releasdir_impl(req)
                    .await
                    .map(|()| unit_response::Result::Ok(()))
                    .unwrap_or_else(|e| unit_response::Result::Errno(e)),
            ),
        }))
    }

    async fn readdir(
        &self,
        req: tonic::Request<proto::fs::ReaddirRequest>,
    ) -> RpcResult<ReaddirResponse> {
        Ok(Response::new(ReaddirResponse {
            result: Some(
                self.readdir_impl(req)
                    .await
                    .map(|inner| readdir_response::Result::Ok(readdir_response::Ok { inner }))
                    .unwrap_or_else(|e| readdir_response::Result::Errno(e)),
            ),
        }))
    }

    async fn open(&self, req: tonic::Request<OpenRequest>) -> RpcResult<HandleResponse> {
        Ok(Response::new(HandleResponse {
            result: Some(
                self.open_impl(req)
                    .await
                    .map(|(fh, flag)| handle_response::Result::Ok(handle_response::Ok { fh, flag }))
                    .unwrap_or_else(|e| handle_response::Result::Errno(e)),
            ),
        }))
    }

    async fn read(&self, req: tonic::Request<ReadRequest>) -> RpcResult<ReadResponse> {
        Ok(Response::new(ReadResponse {
            result: Some(
                self.read_impl(req)
                    .await
                    .map(|data| read_response::Result::Ok(data))
                    .unwrap_or_else(|e| read_response::Result::Errno(e)),
            ),
        }))
    }

    async fn write(&self, req: tonic::Request<WriteRequest>) -> RpcResult<WriteResponse> {
        Ok(Response::new(WriteResponse {
            result: Some(
                self.write_impl(req)
                    .await
                    .map(|written| write_response::Result::Ok(written))
                    .unwrap_or_else(|e| write_response::Result::Errno(e)),
            ),
        }))
    }
}
