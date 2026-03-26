//! Metadata FUSE operations: INIT, LOOKUP, GETATTR, SETATTR, STATFS, FORGET.

use std::os::unix::fs::PermissionsExt;

use crate::hypervisor::fuse::{self, *};
use super::FuseProcessor;

impl FuseProcessor {
    pub(super) fn do_init(&self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if fuse::read_struct::<FuseInitIn>(body).is_none() {
            return fuse::error_response(header.unique, -libc::EIO);
        }

        let init_out = FuseInitOut {
            major: FUSE_KERNEL_VERSION,
            minor: FUSE_KERNEL_MINOR_VERSION,
            max_readahead: 128 * 1024,
            flags: FUSE_BIG_WRITES,
            max_background: 16,
            congestion_threshold: 12,
            max_write: 1 << 20,
            time_gran: 1,
            max_pages: 0,
            map_alignment: 0,
            unused: [0; 8],
        };

        fuse::success_response(header.unique, fuse::as_bytes(&init_out))
    }

    pub(super) fn do_lookup(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let name = match fuse::extract_name(body) {
            Some(n) => n,
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let ino = match self.inodes.lookup(header.nodeid, name) {
            Some(i) => i,
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let path = match self.inodes.get(ino) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => return fuse::error_response(header.unique, -libc::ENOENT),
        };

        let entry = FuseEntryOut {
            nodeid: ino, generation: 0, entry_valid: 1, attr_valid: 1,
            entry_valid_nsec: 0, attr_valid_nsec: 0,
            attr: fuse::metadata_to_fuse_attr(ino, &meta),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&entry))
    }

    pub(super) fn do_getattr(&self, header: &FuseInHeader) -> Vec<u8> {
        let path = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };

        let attr_out = FuseAttrOut {
            attr_valid: 1, attr_valid_nsec: 0, dummy: 0,
            attr: fuse::metadata_to_fuse_attr(header.nodeid, &meta),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&attr_out))
    }

    pub(super) fn do_setattr(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only {
            return fuse::error_response(header.unique, -libc::EROFS);
        }
        let attr_in: FuseSetAttrIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let path = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };

        if attr_in.valid & FATTR_MODE != 0 {
            if let Err(e) = std::fs::set_permissions(&path,
                std::fs::Permissions::from_mode(attr_in.mode)) {
                return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e));
            }
        }
        if attr_in.valid & FATTR_SIZE != 0 {
            let file = match std::fs::OpenOptions::new().write(true).open(&path) {
                Ok(f) => f,
                Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
            };
            if let Err(e) = file.set_len(attr_in.size) {
                return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e));
            }
        }
        if attr_in.valid & (FATTR_UID | FATTR_GID) != 0 {
            let uid = if attr_in.valid & FATTR_UID != 0 { attr_in.uid } else { u32::MAX };
            let gid = if attr_in.valid & FATTR_GID != 0 { attr_in.gid } else { u32::MAX };
            let c_path = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
                Ok(c) => c,
                Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
            };
            if unsafe { libc::chown(c_path.as_ptr(), uid, gid) } != 0 {
                return fuse::error_response(header.unique, -fuse::errno());
            }
        }
        if attr_in.valid & (FATTR_ATIME | FATTR_MTIME) != 0 {
            let c_path = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
                Ok(c) => c,
                Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
            };
            let times = [
                libc::timespec {
                    tv_sec: if attr_in.valid & FATTR_ATIME != 0 { attr_in.atime as i64 } else { 0 },
                    tv_nsec: if attr_in.valid & FATTR_ATIME != 0 { attr_in.atimensec as i64 } else { libc::UTIME_OMIT },
                },
                libc::timespec {
                    tv_sec: if attr_in.valid & FATTR_MTIME != 0 { attr_in.mtime as i64 } else { 0 },
                    tv_nsec: if attr_in.valid & FATTR_MTIME != 0 { attr_in.mtimensec as i64 } else { libc::UTIME_OMIT },
                },
            ];
            if unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) } != 0 {
                return fuse::error_response(header.unique, -fuse::errno());
            }
        }

        self.do_getattr(header)
    }

    pub(super) fn do_statfs(&self, header: &FuseInHeader) -> Vec<u8> {
        let c_path = match std::ffi::CString::new(self.root_path.as_os_str().as_encoded_bytes()) {
            Ok(c) => c,
            Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
            return fuse::error_response(header.unique, -fuse::errno());
        }
        let kstatfs = FuseKStatfs {
            blocks: stat.f_blocks as u64, bfree: stat.f_bfree as u64,
            bavail: stat.f_bavail as u64, files: stat.f_files as u64,
            ffree: stat.f_ffree as u64, bsize: stat.f_bsize as u32,
            namelen: stat.f_namemax as u32, frsize: stat.f_frsize as u32,
            padding: 0, spare: [0; 6],
        };
        fuse::success_response(header.unique, fuse::as_bytes(&kstatfs))
    }

    pub(super) fn do_forget(&mut self, header: &FuseInHeader, body: &[u8]) {
        if let Some(f) = fuse::read_struct::<FuseForgetIn>(body) {
            self.inodes.forget(header.nodeid, f.nlookup);
        }
    }

    pub(super) fn do_batch_forget(&mut self, body: &[u8]) {
        let batch: FuseBatchForgetIn = match fuse::read_struct(body) {
            Some(b) => b,
            None => return,
        };
        let entries_buf = &body[std::mem::size_of::<FuseBatchForgetIn>()..];
        let esz = std::mem::size_of::<FuseForgetOne>();
        for i in 0..batch.count as usize {
            let off = i * esz;
            let e: FuseForgetOne = match fuse::read_struct(&entries_buf[off..]) {
                Some(e) => e,
                None => break,
            };
            self.inodes.forget(e.nodeid, e.nlookup);
        }
    }
}
