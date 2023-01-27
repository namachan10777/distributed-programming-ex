pub mod client;
pub mod server;
pub mod proto {
    pub mod fs {
        use std::{os::unix::prelude::FileTypeExt, time::SystemTime};

        use fuse3::{path::reply::FileAttr, Errno};
        use prost_types::Timestamp;
        use tonic::include_proto;
        include_proto!("fs");

        // TODO: consider before 1970
        pub fn ts_to_systime(ts: Option<Timestamp>) -> fuse3::Result<SystemTime> {
            let ts = ts.ok_or(Errno::from(libc::EACCES))?;
            Ok(SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(ts.seconds as u64)
                + std::time::Duration::from_nanos(ts.nanos as u64))
        }

        pub fn ftype_grpc_to_fuse3(ftype: Filetype) -> fuse3::FileType {
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

        pub fn ftype_std_to_grpc(ftype: std::fs::FileType) -> Filetype {
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

        pub fn attr_grpc_to_fuse3(attr: Attr) -> fuse3::Result<fuse3::path::reply::FileAttr> {
            let kind = ftype_grpc_to_fuse3(attr.kind());
            Ok(FileAttr {
                size: attr.size,
                blocks: attr.blocks,
                atime: ts_to_systime(attr.atime)?,
                mtime: ts_to_systime(attr.mtime)?,
                ctime: ts_to_systime(attr.ctime)?,
                kind,
                perm: attr.perm as u16,
                nlink: attr.nlink,
                uid: attr.uid,
                gid: attr.gid,
                rdev: attr.rdev,
                blksize: attr.blksize,
            })
        }

        pub fn settableattr_fuse3_to_grpc(attr: fuse3::SetAttr) -> fuse3::Result<SettableAttr> {
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
}
