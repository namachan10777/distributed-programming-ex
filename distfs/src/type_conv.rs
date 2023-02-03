pub mod filetype {
    use std::os::unix::prelude::FileTypeExt;

    use crate::proto::fs::Filetype;

    pub fn grpc_to_fuse3(ftype: Filetype) -> fuse3::FileType {
        match ftype {
            Filetype::BlockDevice => fuse3::FileType::BlockDevice,
            Filetype::CharDevice => fuse3::FileType::CharDevice,
            Filetype::Directory => fuse3::FileType::Directory,
            Filetype::RegularFile => fuse3::FileType::RegularFile,
            Filetype::Socket => fuse3::FileType::Socket,
            Filetype::Symlink => fuse3::FileType::Symlink,
            Filetype::NamedPipe => fuse3::FileType::NamedPipe,
        }
    }

    pub fn std_to_grpc(ftype: std::fs::FileType) -> Filetype {
        if ftype.is_dir() {
            Filetype::Directory
        } else if ftype.is_file() {
            Filetype::RegularFile
        } else if ftype.is_symlink() {
            Filetype::Symlink
        } else if ftype.is_socket() {
            Filetype::Socket
        } else if ftype.is_block_device() {
            Filetype::BlockDevice
        } else if ftype.is_char_device() {
            Filetype::CharDevice
        } else if ftype.is_fifo() {
            Filetype::NamedPipe
        } else {
            unreachable!()
        }
    }
}

pub mod timestamp {
    use prost_types::Timestamp;
    use std::time::SystemTime;

    // TODO: consider before 1970
    pub fn grpc_to_system(ts: Timestamp) -> fuse3::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(ts.seconds as u64)
            + std::time::Duration::from_nanos(ts.nanos as u64))
    }
}

pub mod attr {
    use std::{fs::Metadata, os::unix::prelude::MetadataExt};

    use crate::proto::fs::Attr;
    use fuse3::Errno;
    use prost_types::Timestamp;

    pub fn grpc_to_fuse3(attr: Attr) -> fuse3::Result<fuse3::path::reply::FileAttr> {
        let kind = attr.kind();
        Ok(fuse3::path::reply::FileAttr {
            size: attr.size,
            blocks: attr.blocks,
            atime: super::timestamp::grpc_to_system(attr.atime.ok_or(Errno::from(libc::EACCES))?)?,
            mtime: super::timestamp::grpc_to_system(attr.mtime.ok_or(Errno::from(libc::EACCES))?)?,
            ctime: super::timestamp::grpc_to_system(attr.ctime.ok_or(Errno::from(libc::EACCES))?)?,
            kind: super::filetype::grpc_to_fuse3(kind),
            perm: attr.perm as u16,
            nlink: attr.nlink,
            uid: attr.uid,
            gid: attr.gid,
            rdev: attr.rdev,
            blksize: attr.blksize,
        })
    }

    pub fn metadata_to_grpc(meta: Metadata) -> Attr {
        Attr {
            size: meta.len(),
            blocks: meta.blocks(),
            atime: Some(Timestamp {
                seconds: meta.atime(),
                nanos: 0,
            }),
            mtime: Some(Timestamp {
                seconds: meta.mtime(),
                nanos: 0,
            }),
            ctime: Some(Timestamp {
                seconds: meta.ctime(),
                nanos: 0,
            }),
            kind: super::filetype::std_to_grpc(meta.file_type()).into(),
            perm: meta.mode() & 0o777,
            nlink: meta.nlink() as u32,
            uid: meta.uid(),
            gid: meta.gid(),
            // FIXME
            flag: 0,
            rdev: meta.rdev() as u32,
            blksize: meta.blksize() as u32,
        }
    }
}

pub mod set_attr {
    use crate::proto::fs::SettableAttr;
    use prost_types::Timestamp;

    pub fn grpc_to_fuse3(attr: SettableAttr) -> fuse3::Result<fuse3::SetAttr> {
        Ok(fuse3::SetAttr {
            mode: attr.mode,
            uid: attr.uid,
            gid: attr.gid,
            size: attr.size,
            lock_owner: attr.lock_owner,
            atime: attr
                .atime
                .map(|ts| fuse3::Timestamp::new(ts.seconds, ts.nanos as u32)),
            mtime: attr
                .mtime
                .map(|ts| fuse3::Timestamp::new(ts.seconds, ts.nanos as u32)),
            ctime: attr
                .ctime
                .map(|ts| fuse3::Timestamp::new(ts.seconds, ts.nanos as u32)),
        })
    }

    pub fn fuse3_to_grpc(attr: fuse3::SetAttr) -> fuse3::Result<SettableAttr> {
        Ok(SettableAttr {
            mode: attr.mode,
            uid: attr.uid,
            gid: attr.gid,
            size: attr.size,
            lock_owner: attr.lock_owner,
            atime: attr.atime.map(|ts| Timestamp {
                seconds: ts.sec,
                nanos: ts.nsec as i32,
            }),
            mtime: attr.mtime.map(|ts| Timestamp {
                seconds: ts.sec,
                nanos: ts.nsec as i32,
            }),
            ctime: attr.ctime.map(|ts| Timestamp {
                seconds: ts.sec,
                nanos: ts.nsec as i32,
            }),
        })
    }
}
