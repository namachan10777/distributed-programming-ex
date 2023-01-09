use std::ffi::OsStr;

use fuse3::path::prelude::*;

pub struct Distfs;

impl Distfs {
    pub fn new<P: AsRef<std::path::Path>>(_target: P) -> Self {
        unimplemented!()
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
        _parent: &OsStr,
        _name: &OsStr,
    ) -> fuse3::Result<ReplyEntry> {
        unimplemented!()
    }
    async fn forget(&self, _req: Request, _parent: &OsStr, _nlookup: u64) {}
    async fn getattr(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: Option<u64>,
        _flags: u32,
    ) -> fuse3::Result<ReplyAttr> {
        unimplemented!()
    }
    async fn setattr(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: Option<u64>,
        _set_attr: SetAttr,
    ) -> fuse3::Result<ReplyAttr> {
        unimplemented!()
    }
    async fn mkdir(
        &self,
        _req: Request,
        _parent: &OsStr,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
    ) -> fuse3::Result<ReplyEntry> {
        unimplemented!()
    }
    async fn unlink(&self, _req: Request, _parent: &OsStr, _name: &OsStr) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn readlink(&self, _req: Request, _path: &OsStr) -> fuse3::Result<ReplyData> {
        unimplemented!()
    }
    async fn symlink(
        &self,
        _req: Request,
        _parent: &OsStr,
        _name: &OsStr,
        _link_path: &OsStr,
    ) -> fuse3::Result<ReplyEntry> {
        unimplemented!()
    }
    async fn rmdir(&self, _req: Request, _parent: &OsStr, _name: &OsStr) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn rename(
        &self,
        _req: Request,
        _origin_parent: &OsStr,
        _origin_name: &OsStr,
        _parent: &OsStr,
        _name: &OsStr,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn open(&self, _req: Request, _path: &OsStr, _flags: u32) -> fuse3::Result<ReplyOpen> {
        unimplemented!()
    }
    async fn read(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _offset: u64,
        _size: u32,
    ) -> fuse3::Result<ReplyData> {
        unimplemented!()
    }
    async fn write(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _offset: u64,
        _data: &[u8],
        _flags: u32,
    ) -> fuse3::Result<ReplyWrite> {
        unimplemented!()
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
        unimplemented!()
    }
    async fn fsync(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _datasync: bool,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn flush(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _lock_owner: u64,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn access(&self, _req: Request, _path: &OsStr, _mask: u32) -> fuse3::Result<()> {
        unimplemented!()
    }
    async fn create(
        &self,
        _req: Request,
        _parent: &OsStr,
        _name: &OsStr,
        _mode: u32,
        _flags: u32,
    ) -> fuse3::Result<ReplyCreated> {
        unimplemented!()
    }
    async fn batch_forget(&self, _req: Request, _paths: &[&OsStr]) {
        unimplemented!()
    }

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
        unimplemented!()
    }

    async fn opendir(&self, _req: Request, _path: &OsStr, _flags: u32) -> fuse3::Result<ReplyOpen> {
        unimplemented!()
    }

    async fn releasedir(
        &self,
        _req: Request,
        _path: &OsStr,
        _fh: u64,
        _flags: u32,
    ) -> fuse3::Result<()> {
        unimplemented!()
    }

    async fn readdirplus(
        &self,
        _req: Request,
        _parent: &OsStr,
        _fh: u64,
        _offset: u64,
        _lock_owner: u64,
    ) -> fuse3::Result<ReplyDirectoryPlus<Self::DirEntryPlusStream>> {
        unimplemented!()
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
        unimplemented!()
    }
    async fn lseek(
        &self,
        _req: Request,
        _path: Option<&OsStr>,
        _fh: u64,
        _offset: u64,
        _whence: u32,
    ) -> fuse3::Result<ReplyLSeek> {
        unimplemented!()
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
        unimplemented!()
    }
}
