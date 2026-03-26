//! FUSE protocol constants and wire-format structs.
//!
//! All structs are `#[repr(C)]` matching `include/uapi/linux/fuse.h`.

// ---------------------------------------------------------------------------
// FUSE protocol constants
// ---------------------------------------------------------------------------

pub const FUSE_KERNEL_VERSION: u32 = 7;
pub const FUSE_KERNEL_MINOR_VERSION: u32 = 31;

// Opcodes
pub const FUSE_LOOKUP: u32 = 1;
pub const FUSE_FORGET: u32 = 2;
pub const FUSE_GETATTR: u32 = 3;
pub const FUSE_SETATTR: u32 = 4;
pub const FUSE_READLINK: u32 = 22;
pub const FUSE_SYMLINK: u32 = 6;
pub const FUSE_MKNOD: u32 = 8;
pub const FUSE_MKDIR: u32 = 9;
pub const FUSE_UNLINK: u32 = 10;
pub const FUSE_RMDIR: u32 = 11;
pub const FUSE_RENAME: u32 = 12;
pub const FUSE_LINK: u32 = 13;
pub const FUSE_OPEN: u32 = 14;
pub const FUSE_READ: u32 = 15;
pub const FUSE_WRITE: u32 = 16;
pub const FUSE_STATFS: u32 = 17;
pub const FUSE_RELEASE: u32 = 18;
pub const FUSE_FSYNC: u32 = 20;
pub const FUSE_FSYNCDIR: u32 = 21;
pub const FUSE_FLUSH: u32 = 25;
pub const FUSE_INIT: u32 = 26;
pub const FUSE_OPENDIR: u32 = 27;
pub const FUSE_READDIR: u32 = 28;
pub const FUSE_RELEASEDIR: u32 = 29;
pub const FUSE_CREATE: u32 = 35;
pub const FUSE_BATCH_FORGET: u32 = 42;
pub const FUSE_RENAME2: u32 = 45;
pub const FUSE_LSEEK: u32 = 46;

// INIT flags
pub const FUSE_BIG_WRITES: u32 = 1 << 5;

// SETATTR valid bits
pub const FATTR_MODE: u32 = 1 << 0;
pub const FATTR_UID: u32 = 1 << 1;
pub const FATTR_GID: u32 = 1 << 2;
pub const FATTR_SIZE: u32 = 1 << 3;
pub const FATTR_ATIME: u32 = 1 << 4;
pub const FATTR_MTIME: u32 = 1 << 5;

// File type masks
pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFCHR: u32 = 0o020000;

// DT_* values for dirent type field
pub const DT_UNKNOWN: u32 = 0;
pub const DT_REG: u32 = 8;
pub const DT_DIR: u32 = 4;
pub const DT_LNK: u32 = 10;
pub const DT_BLK: u32 = 6;
pub const DT_CHR: u32 = 2;

// ---------------------------------------------------------------------------
// Wire-format structs
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseInHeader {
    pub len: u32,
    pub opcode: u32,
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseOutHeader {
    pub len: u32,
    pub error: i32,
    pub unique: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FuseAttr {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atimensec: u32,
    pub mtimensec: u32,
    pub ctimensec: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseEntryOut {
    pub nodeid: u64,
    pub generation: u64,
    pub entry_valid: u64,
    pub attr_valid: u64,
    pub entry_valid_nsec: u32,
    pub attr_valid_nsec: u32,
    pub attr: FuseAttr,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseAttrOut {
    pub attr_valid: u64,
    pub attr_valid_nsec: u32,
    pub dummy: u32,
    pub attr: FuseAttr,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseInitIn {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseInitOut {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
    pub max_background: u16,
    pub congestion_threshold: u16,
    pub max_write: u32,
    pub time_gran: u32,
    pub max_pages: u16,
    pub map_alignment: u16,
    pub unused: [u32; 8],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseOpenIn { pub flags: u32, pub open_flags: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseOpenOut { pub fh: u64, pub open_flags: u32, pub padding: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseReadIn {
    pub fh: u64, pub offset: u64, pub size: u32, pub read_flags: u32,
    pub lock_owner: u64, pub flags: u32, pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseWriteIn {
    pub fh: u64, pub offset: u64, pub size: u32, pub write_flags: u32,
    pub lock_owner: u64, pub flags: u32, pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseWriteOut { pub size: u32, pub padding: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseCreateIn { pub flags: u32, pub mode: u32, pub umask: u32, pub open_flags: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseMkdirIn { pub mode: u32, pub umask: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseMknodIn { pub mode: u32, pub rdev: u32, pub umask: u32, pub padding: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseSetAttrIn {
    pub valid: u32, pub padding: u32, pub fh: u64, pub size: u64,
    pub lock_owner: u64, pub atime: u64, pub mtime: u64, pub ctime: u64,
    pub atimensec: u32, pub mtimensec: u32, pub ctimensec: u32, pub mode: u32,
    pub unused4: u32, pub uid: u32, pub gid: u32, pub unused5: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseRenameIn { pub newdir: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseRename2In { pub newdir: u64, pub flags: u32, pub padding: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseLinkIn { pub oldnodeid: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseForgetIn { pub nlookup: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseBatchForgetIn { pub count: u32, pub dummy: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseForgetOne { pub nodeid: u64, pub nlookup: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseReleaseIn { pub fh: u64, pub flags: u32, pub release_flags: u32, pub lock_owner: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseFsyncIn { pub fh: u64, pub fsync_flags: u32, pub padding: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseFlushIn { pub fh: u64, pub unused: u32, pub padding: u32, pub lock_owner: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseKStatfs {
    pub blocks: u64, pub bfree: u64, pub bavail: u64, pub files: u64, pub ffree: u64,
    pub bsize: u32, pub namelen: u32, pub frsize: u32, pub padding: u32, pub spare: [u32; 6],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseLseekIn { pub fh: u64, pub offset: u64, pub whence: u32, pub padding: u32 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseLseekOut { pub offset: u64 }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseDirent { pub ino: u64, pub off: u64, pub namelen: u32, pub type_: u32 }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn fuse_in_header_size() { assert_eq!(std::mem::size_of::<FuseInHeader>(), 40); }
    #[test] fn fuse_out_header_size() { assert_eq!(std::mem::size_of::<FuseOutHeader>(), 16); }
    #[test] fn fuse_attr_size() { assert_eq!(std::mem::size_of::<FuseAttr>(), 88); }
    #[test] fn fuse_entry_out_size() { assert_eq!(std::mem::size_of::<FuseEntryOut>(), 128); }
    #[test] fn fuse_attr_out_size() { assert_eq!(std::mem::size_of::<FuseAttrOut>(), 104); }
    #[test] fn fuse_init_in_size() { assert_eq!(std::mem::size_of::<FuseInitIn>(), 16); }
    #[test] fn fuse_init_out_size() { assert_eq!(std::mem::size_of::<FuseInitOut>(), 64); }
    #[test] fn fuse_open_in_size() { assert_eq!(std::mem::size_of::<FuseOpenIn>(), 8); }
    #[test] fn fuse_open_out_size() { assert_eq!(std::mem::size_of::<FuseOpenOut>(), 16); }
    #[test] fn fuse_read_in_size() { assert_eq!(std::mem::size_of::<FuseReadIn>(), 40); }
    #[test] fn fuse_write_in_size() { assert_eq!(std::mem::size_of::<FuseWriteIn>(), 40); }
    #[test] fn fuse_write_out_size() { assert_eq!(std::mem::size_of::<FuseWriteOut>(), 8); }
    #[test] fn fuse_create_in_size() { assert_eq!(std::mem::size_of::<FuseCreateIn>(), 16); }
    #[test] fn fuse_mkdir_in_size() { assert_eq!(std::mem::size_of::<FuseMkdirIn>(), 8); }
    #[test] fn fuse_mknod_in_size() { assert_eq!(std::mem::size_of::<FuseMknodIn>(), 16); }
    #[test] fn fuse_setattr_in_size() { assert_eq!(std::mem::size_of::<FuseSetAttrIn>(), 88); }
    #[test] fn fuse_rename_in_size() { assert_eq!(std::mem::size_of::<FuseRenameIn>(), 8); }
    #[test] fn fuse_rename2_in_size() { assert_eq!(std::mem::size_of::<FuseRename2In>(), 16); }
    #[test] fn fuse_link_in_size() { assert_eq!(std::mem::size_of::<FuseLinkIn>(), 8); }
    #[test] fn fuse_forget_in_size() { assert_eq!(std::mem::size_of::<FuseForgetIn>(), 8); }
    #[test] fn fuse_batch_forget_in_size() { assert_eq!(std::mem::size_of::<FuseBatchForgetIn>(), 8); }
    #[test] fn fuse_forget_one_size() { assert_eq!(std::mem::size_of::<FuseForgetOne>(), 16); }
    #[test] fn fuse_release_in_size() { assert_eq!(std::mem::size_of::<FuseReleaseIn>(), 24); }
    #[test] fn fuse_fsync_in_size() { assert_eq!(std::mem::size_of::<FuseFsyncIn>(), 16); }
    #[test] fn fuse_flush_in_size() { assert_eq!(std::mem::size_of::<FuseFlushIn>(), 24); }
    #[test] fn fuse_kstatfs_size() { assert_eq!(std::mem::size_of::<FuseKStatfs>(), 80); }
    #[test] fn fuse_lseek_in_size() { assert_eq!(std::mem::size_of::<FuseLseekIn>(), 24); }
    #[test] fn fuse_lseek_out_size() { assert_eq!(std::mem::size_of::<FuseLseekOut>(), 8); }
    #[test] fn fuse_dirent_size() { assert_eq!(std::mem::size_of::<FuseDirent>(), 24); }
}
