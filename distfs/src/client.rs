use std::ffi::OsStr;

use fuse3::path::prelude::*;

pub struct Distfs;

impl Distfs {
    pub fn new<P: AsRef<std::path::Path>>(target: P) -> Self {
        unimplemented!()
    }
}

#[async_trait::async_trait]
impl PathFilesystem for Distfs {
    type DirEntryStream = futures_util::stream::Empty<fuse3::Result<DirectoryEntry>>;
    type DirEntryPlusStream =
        futures_util::stream::Iter<std::vec::IntoIter<fuse3::Result<DirectoryEntryPlus>>>;

    async fn init(&self, _req: Request) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn destroy(&self, _req: Request) {}
    async fn lookup(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
    ) -> fuse3::Result<ReplyEntry> {
        unimplemented!()
    }
    async fn forget(&self, _req: Request, _parent: &OsStr, _nlookup: u64) {}
    async fn getattr(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        _fh: Option<u64>,
        _flags: u32,
    ) -> fuse3::Result<ReplyAttr> {
        unimplemented!()
    }
    async fn setattr(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        _fh: Option<u64>,
        set_attr: SetAttr,
    ) -> fuse3::Result<ReplyAttr> {
        unimplemented!()
    }
    async fn mkdir(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
        mode: u32,
        _umask: u32,
    ) -> fuse3::Result<ReplyEntry> {
        unimplemented!()
    }
    async fn unlink(&self, _req: Request, parent: &OsStr, name: &OsStr) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn rmdir(&self, _req: Request, parent: &OsStr, name: &OsStr) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn rename(
        &self,
        _req: Request,
        origin_parent: &OsStr,
        origin_name: &OsStr,
        parent: &OsStr,
        name: &OsStr,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn open(&self, _req: Request, path: &OsStr, flags: u32) -> fuse3::Result<ReplyOpen> {
        unimplemented!()
    }
    async fn read(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        _fh: u64,
        offset: u64,
        size: u32,
    ) -> fuse3::Result<ReplyData> {
        unimplemented!()
    }
    async fn write(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        data: &[u8],
        flags: u32,
    ) -> fuse3::Result<ReplyWrite> {
        unimplemented!()
    }
    async fn release(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: u64,
        flags: u32,
        lock_owner: u64,
        flush: bool,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn fsync(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: u64,
        datasync: bool,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn flush(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: u64,
        lock_owner: u64,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn access(&self, _req: Request, path: &OsStr, _mask: u32) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn create(
        &self,
        _req: Request,
        parent: &OsStr,
        name: &OsStr,
        mode: u32,
        flags: u32,
    ) -> fuse3::Result<ReplyCreated> {
        unimplemented!()
    }
    async fn batch_forget(&self, req: Request, paths: &[&OsStr]) {
        unimplemented!()
    }

    #[cfg(target_os = "linux")]
    async fn fallocate(
        &self,
        req: Request,
        path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        length: u64,
        mode: u32,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }

    async fn readdirplus(
        &self,
        req: Request,
        parent: &OsStr,
        fh: u64,
        offset: u64,
        lock_owner: u64,
    ) -> fuse3::Result<ReplyDirectoryPlus<Self::DirEntryPlusStream>> {
        unimplemented!()
    }
    async fn rename2(
        &self,
        req: Request,
        origin_parent: &OsStr,
        origin_name: &OsStr,
        parent: &OsStr,
        name: &OsStr,
        flags: u32,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn lseek(
        &self,
        req: Request,
        path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        whence: u32,
    ) -> fuse3::Result<ReplyLSeek> {
        unimplemented!()
    }
    async fn copy_file_range(
        &self,
        req: Request,
        from_path: Option<&OsStr>,
        fh_in: u64,
        offset_in: u64,
        to_path: Option<&OsStr>,
        fh_out: u64,
        offset_out: u64,
        length: u64,
        flags: u64,
    ) -> fuse3::Result<ReplyCopyFileRange> {
        unimplemented!()
    }
}
